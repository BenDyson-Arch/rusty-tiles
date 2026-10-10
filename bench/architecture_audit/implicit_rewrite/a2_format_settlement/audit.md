# A2 format/consumer decision

## Decision and admitted profile

Retain owned roots only as an interoperability candidate for the full byte-preserving operation, with
source-child ordinals, mandatory authoritative tile bounds/error rows, and a
declared **terminal-occupancy interoperability profile**. Continue its bounded
consumer proof. Do not silently label the profile generic standards conformance
or introduce a production writer before coordinator settlement and nonauthor
review. The coordinator has explicitly withheld authorization for a production
A2 replacement claiming valid 3D Tiles. These observations do not authorize the
terminal-template representation under that claim. Advance independent private
payload/archive ownership foundations while separate review settles the next
scope. No public browser work is currently authorized in this lane.

Every authored source node owns a separate implicit root. Leaves use one level;
internal nodes use two. Root payload slots are available only at level 0. A
dedicated external-JSON template slot is unavailable at level 0 and available only
at the terminal level 1 addresses. All child-subtree bits are zero. The external
document root owns the original child transform and payloads/descendants; the
terminal link remains in its parent root's frame. Internal empty source nodes are
preserved. Slots index contents/availability together, including multiple root
payloads and a JSON slot. Child ordinal capacity is explicit: at most 4 or 8.

All source payload slots must be present at their owning root, in source array
order, with one availability layer per template; no extra or orphan slot is
admitted. Terminal links have exactly one available external document and no
available payloads or implicit children. The reader checks actual resolved
document ancestry for cycles before interpreting a repeated document. It checks
resource roles against GLB header/position bytes or parsed external tileset JSON,
not a URI suffix, and external class definitions against the top-level schema.
All valid roots' external slot availability is zero. These fixture probes cover
GLB payloads; other payload kinds require their independent registered decoders,
bounded resource resolution and coherence acceptance, not a guessed type.

Two reader-only controls per scheme replace a JSON link with actual GLB bytes,
or redirect a child's template back to its active external document. All four
are independently rejected for the intended type/cycle reason. They are declared
in-memory controls, not additional executed client/validator cases.

The terminal-occupancy interpretation follows availability-defined association
of resources and the core rule that an external-reference tile has no children.
Its limit is the separate literal prohibition on an implicit root's content URI:
the root manifest still contains the JSON template. The text does not explicitly
exempt a template whose level-0 slot is unavailable. Neither this audit nor a
tolerant client invents that exemption as settled normative meaning.
[Official implicit tiling, root/content availability](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/ImplicitTiling/README.adoc),
[official external tilesets](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc#external-tilesets).

## Settled spatial and ownership policy

Retain source-order addresses. The primary subdivision rules allow metadata to
override calculated bounding volumes and errors while retaining spatial coherence.
There is no additional nominal-cell containment/center-match condition after that
override. Every available row in the profile therefore supplies actual bounds;
coordinates identify address/availability membership and cannot certify geometry.
Do not infer spatial location from a source child's center or require distinct
centers. This is a profile using standard override semantics, not a claim that
all coordinate-indexed GIS consumers support arbitrary overridden placement.
[Official subdivision and tile metadata](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/ImplicitTiling/README.adoc#subdivision-rules).

The five-node candidates contain two coincident child centers/boxes with different
identities, errors, proxy content and descendant ownership. Both independent raw
reading and actual client materialization preserve them. Generated terminal link
boxes are the child source box transformed into the parent-local frame. Their
computed frame equals the parent's computed frame. Loading the external root
then applies the source child's transform exactly once. Nonidentity root rotation
and translation, nested translations, multi-slot roots and an empty internal node
all pass. A deliberately added child translation changes the actual matrices and
is rejected independently. Core external-tree composition requires both ancestor
and external root transforms; core coherence requires child content inside its
parent's box. The reader additionally checks every ancestor's box.
[Official transforms and coherence](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc).

This settles the unnecessary cell/center rejection independently of the terminal
template question. The earlier accepted real-source document subset examined
11 actual point/vector sources and 6 successful legacy outputs. None of those
11 met exact whole-box nominal-cell fit, although the successful output rows,
aliases and frame compositions passed. Flat point thickness and tiny shallow
vector edge excesses are useful counterexamples to a nominal-cell admission
policy; they are not permissions to ignore decoded geometry. Deep/tight and
fragmented/array sources remain subject to bounded branching, supported resources,
all-slot ownership and independently decoded content coherence. This settlement
does not rerun or expand that real-source matrix; see the existing paired
[real document receipt](../a2_semantics/real-results.json) and separate source
geometry audit. Archive/resource/provenance claims stay at their owning boundaries.

## Actual client result and sensitive limits

Pinned Cesium `1.146.0` / engine `26.4.0` materializes both Q/O candidate trees,
every payload slot and every terminal JSON link. Its four inspected primary
files exactly match the executed bytes at commit
`df52c781de3491a4b76839d420f7ca90a032efb6`; complete engine/core JS tree hashes
are in the receipt. The independent reader agrees with the actual matrices and
raw rows. Direct factory execution is stronger than reading production output,
but is not public viewer selection/feature evidence.
[Pinned client materializer](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Implicit3DTileContent.js),
[pinned tile construction](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Cesium3DTile.js).

The client also loads the root-available JSON corruption into a root that already
has children. This control is rejected by the independent occupancy rule. Thus
client success demonstrably cannot establish the root prohibition or terminal
leaf requirement. The control is not evidence that the corrupt representation is
supported. It is an intentionally forbidden occupancy case.

For the no-external-template alternative, actual metadata returns the declared
`TILE_TRANSFORM` matrix translating `(9,10,11)`, while the actual computed transform
remains identity in both schemes. This is an executed consumer limitation, not
missing/malformed metadata. The primary semantic is equivalent to tile transform;
the client construction path establishes transforms before attaching tile
metadata and does not apply this semantic.
[Official transform semantic](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/Metadata/Semantics/README.adoc),
[pinned materialization code](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Implicit3DTileContent.js).

## Official validator/tools evidence

The current inspected official validator commit is
`7fa62c5f792069b077f174b477aab85dd7fecf22`, package version `0.6.1`.
`TileValidator.validateImplicitTilesetRoot` explicitly quotes the external URI
prohibition, followed by a TODO; it does not validate that condition there.
No official validator executable was installed locally and no dependency
installation was performed. These are static observations, not an executed
validator result. Even a future green run of this implementation cannot close
the quoted TODO by itself.
[Official validator implementation](https://github.com/CesiumGS/3d-tiles-validator/blob/7fa62c5f792069b077f174b477aab85dd7fecf22/src/validation/TileValidator.ts).

The inspected official tools commit is
`4ca692eb16a9c7db21ec99e2aacc32645ce92f28`, package version `0.5.4`.
`ImplicitTraversedTile.getRawContents` loops availability layers but reads
`root.content?.uri` for each available layer, rather than the associated
`root.contents[slot]`. It is not an independent multi-slot truth oracle for this
profile. Conversely `MetadataSemanticOverrides` assigns the `TILE_TRANSFORM`
value to `tile.transform`. This demonstrates a statically implemented semantic
in that tooling, not rendering support in the intended client or an executed
tools/validator run.
[Official traverser](https://github.com/CesiumGS/3d-tiles-tools/blob/4ca692eb16a9c7db21ec99e2aacc32645ce92f28/src/tilesets/traversal/ImplicitTraversedTile.ts),
[official semantic overrides](https://github.com/CesiumGS/3d-tiles-tools/blob/4ca692eb16a9c7db21ec99e2aacc32645ce92f28/src/tilesets/traversal/MetadataSemanticOverrides.ts).

## Finite alternatives and dispositions

| Representation/component | Disposition | Preservation and proof consequence |
| --- | --- | --- |
| Source-order owned roots with terminal JSON templates | Retain as declared interoperability profile candidate | All local frames and payload bytes preserved; literal root-template standards claim gate and public viewer gate remain |
| Legacy center/cell fitting, epsilon padding and midpoint routing | Replace | Mandatory actual bound rows and ordinal address ownership remove that admission fiction |
| Single implicit tree, direct GLB slots, `TILE_TRANSFORM` rows | Unsupported in intended pinned client | Removes JSON templates without rewriting payloads, but executed placement is wrong because semantic is ignored; another fully verified consumer would be a different support profile |
| Single implicit tree with rewritten/baked GLB transforms | Remove from byte-preserving product scope | Alters payload bytes and potentially quantization/resources/features; would require a separate transform product and geometry proof |
| Explicit internal skeleton plus one-level implicit source leaves | Separate compliant partial-implicit product | Keeps original internal proxy content/ordering/relative transforms; leaf roots have only direct payload templates and zero child-subtrees; does not convert the internal hierarchy to implicit addressing |
| Production expander or current tools traverser as semantic truth | Remove | Shared or incomplete implementation cannot independently certify source meaning/all slots |
| Root-available external JSON despite tolerant client | Unsupported | Profile forbids it; sensitive control proves client lacks enforcement |
| Inspector whole-child-box containment | Rework separately | Admission must distinguish actual content coherence from nominal or whole-box containment; no orphan admission |

If separate review selects further consumer proof, the smallest execution is
the two candidate five-node fixtures in the pinned public Cesium viewer, under
coordinator CPU serialization, using two camera/SSE
states for coarse proxy and fine descendant selection. Record requested/rendered
content identities for every slot, public pick/feature behavior, materialized
ancestor bounds and the source IDs of coincident branches. Run a wrong-frame/
ownership control to prove the public checks are sensitive. Do not install or
launch a browser concurrently with a heavy build/converter run. The candidate
folders from this capture can be served unchanged; additions needed for feature
queries must have their own authored/pinned fixture receipt.

For a standards-certified claim, close the terminal-template gate through an
authoritative clarification that addresses unavailable level-0 JSON template
slots, or select an unambiguous different representation/product explicitly.
A schema pass, validator TODO, tolerant client or absence of a rejection is
insufficient. This finite standards obligation does not invalidate the settled
ownership/frame policy or prevent separate dependency-ready foundation work.
