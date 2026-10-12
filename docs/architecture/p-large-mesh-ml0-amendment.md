# P implicit layout: large-mesh tiler amendment (ML0, P-A1 to P-A7)

Status: **DRAFT, light process** (one fresh nonauthor review; USER process decision 2026-10-12).
Written by the ML0 author on design v6.1 and the coordinator's rulings of 2026-10-12. Nothing here
records acceptance.

This note amends [p-impl-layout-contract.md](p-impl-layout-contract.md) (frozen v3 plus the
delta-review amendments; sha256 `1f356799…d28a`), which stays byte-identical. Where they disagree
on an item below, this note governs. ML3 part A (`implicit/tileset.rs`) implements it.

**Base:** `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26`. **Design:** large-mesh v6.1
(`design.md` `0bf9fd19…441b`) §2.4, §2.5, §2.6, §9.1; reviews `698b3b61…c4ef` (v5: B2, B3, B4,
E1) and `3f2c0583…ae6d` (v6: N4, N5). **Umbrella:** [f1d3-large-mesh-contract.md](f1d3-large-mesh-contract.md) §4, §5.

## Unchanged

The layout (skeleton, implicit roots with one subtree, explicit leaves, link tiles, explicit
external documents); `S` = 4; `DOC_NODES` = 16,384, `DOC_BYTES` = 2 MiB, `DOC_LEVELS` = 10;
the `tileset.json` reserve of 64 nodes and 4 KiB (D-3); invariants P1–P12; the rebase and its
inverse (D-4); the failure list of §4.4; the I1 interface (§4.6); reuse (§4.7). **Point-cloud,
vector and legacy-mesh output stay byte-identical** (P-A3).

---

## P-A1. The large-mesh producer joins the domain

Add to §4.3 "Producers": **`mesh-local-to-3tz` (F1d3), OCTREE**, with:

- one content slot (`.glb`) on every tile, leaves and proxies alike. No P-A item uses P §4.3's
  b3dm suffix; the glTF-only slice (C1 amendment A-C1-6) removes b3dm from P's domain;
- at most 8 children with distinct slots: the child's octant, or P-A5;
- **frames (coordinator ruling on O-D1/O-P-1: accepted).** The root tile's `transform` carries
  the F1c1 placement composed with a translation by the global lattice origin `O`.
  Translation-only transforms appear below the root **only on explicit tiles and implicit
  roots**, each an integer multiple of the coarsest lattice step `s_0`, used where payload offsets
  would otherwise reach 2^24. Implicit tiles carry none. So δ(t) = 0 for every tile, every
  implicit payload is a byte copy (D-4: a zero δ is a byte copy for any suffix), and decoded
  positions stay exact dyadic values shared across tiles.

**Reconciliation with v6.1.** Four v6.1 sentences assume a nonzero rebase. This amendment and the
umbrella (§4) supersede them for the mesh producer:
- §3.1 `ε_rebase = 2·ulp(...)` is 0;
- §3.4 "a forest wider than `2^24·s_d` uses the per-forest rebase (P §4.1)" becomes the
  lattice-aligned implicit-root translations above (P's δ is a per-tile sum of child translations
  below an implicit root, not a per-forest shift);
- §7 "the node TRS carries the lattice origin and step" becomes: the node TRS carries the step and
  an integer-multiple offset; the origin is in the root transform;
- §2.7 stage 2's "two copies of one implicit subtree's payloads for the forest rebase" term is 0
  (no rebase, and names are final before payloads are written, P-A2).

## P-A2. Streaming input in two phases

`write_forest` today takes the finished hierarchy H in memory. For the large-mesh producer H has
up to about 1.6×10^8 tiles (U2), so the writer takes two streams. Both list the nodes of H in
**post-order with children in ascending slot order**. The producer obtains this order by a k-way
merge of its per-depth, Morton-ordered spills (one cursor per depth, at most 32).

1. **Topology stream** (after pass 5a): per node, its identity (umbrella §5), depth, slot and
   whether it is a leaf. From it the writer computes, in one pass with a stack of at most 32
   entries:
   - `h(n)` (P §4.1), and so every node's class (skeleton, explicit leaf, implicit root,
     implicit);
   - the preorder index of every implicit root (P-A4);
   - every implicit tile's root and local coordinates `(level, x, y, z)`, and so the **final
     member name** of every payload.

   It returns the name map as a spill keyed by stream position. The producer writes each payload
   into the archive under its final name as soon as it is assembled (pass 7), in any order. There
   is no second payload copy.
2. **Record stream** (after pass 7): per node, `B(t)`, `e(t)`, `refine`, extras and its content
   `(member, bytes)`, in the same order. The writer emits subtree members and documents from it.

**The hierarchy is final at pass 5a.** Design v6.1 E49 builds the whole cell tree there,
including packing, run sub-hierarchies and the byte-budget leaf test, and N5 guarantees that no
later pass splits a leaf (lossy profiles drop a mip level instead; `lossless` refuses). So phase 1
runs right after pass 5a.

**Bounded state.** Phase 1: the stack (32 entries) plus the name spill. Phase 2: an implicit
root's subtree is a contiguous run of the post-order stream ending at the root, so at most one
subtree is open, at most 585 records (OCTREE, `S` = 4); plus one document accumulator per path
level (P-A3). `M_layout ≤ D_open × DOC_BYTES ≤ 32 × 2 MiB` plus one subtree buffer.

**Disagreement** between the two streams (a node count, identity, slot or leaf flag differs) is an
internal error (`InvalidState`), never a verdict about the input.

## P-A3. Bottom-up document packing, for mesh output only (coordinator ruling O-P-2)

P §4.1 packs documents breadth-first from the root, which needs the whole skeleton in memory.
**For the large-mesh producer only**, documents are packed bottom-up, deciding each cut when a
skeleton child completes, with one open accumulator per path level:

1. The accumulator of skeleton tile `t` holds `t`'s unit (its explicit leaves and implicit-root
   tiles, which are always kept with `t`) and the fragments of its accepted skeleton children.
2. When skeleton child `c` of `t` completes with fragment `F(c)`, it is **accepted** into `t`'s
   accumulator if the accumulator plus `F(c)` plus a **reserve** stays within `DOC_NODES` value
   nodes and `DOC_BYTES` serialized bytes, and `F(c)` spans at most `DOC_LEVELS − 1` skeleton
   levels. The reserve is an upper bound on `t`'s own tile object plus one link tile for each of
   `t`'s skeleton children not yet completed. The topology stream gives that count, and the
   writer fixes the bounds as profile constants from the widest f64 serialization.
3. Otherwise `c` is **cut**: `F(c)` is written as document `tileset-d{m}.json` with root `c`
   (without its transform), `m` counting from 1 in cut order, and a link tile for `c` (P §4.1) is
   appended to `t`'s accumulator.
4. The root's accumulator is `tileset.json`, with the D-3 reserve added to its reserve.
5. A single unit larger than the budget is the existing typed writer error.

**Point, vector and legacy mesh keep breadth-first packing and stay byte-identical** in 0.4.0, so
there is no vector reuse-identity break from the layout and no need to re-run P §5.2–§5.3 for
them. The packing rule is selected by the producer, not by the data, so P §4.1's "no
data-dependent alternative layout" still holds. **Recorded as a later option:** moving the other
producers to bottom-up packing, with their own request-count and Cesium evidence.

P7 holds for the mesh producer: the bytes are a function of H and the profile constants, and are
independent of the stream chunking, `R`, `F` and `--jobs`. P8 holds by construction (every
accept is checked). P5 and P6 are unchanged.

## P-A4. Implicit-root names stay preorder indices

P §4.1 R-7 names root `r` by its preorder index `n` in H. It is computable in the post-order
stream: `n = c(r) + depth(r)`, where `c(r)` is the number of nodes the stream emitted before the
first node of `r`'s subtree. **Reconciliation with v6.1 §9.1** ("preorder ids … assigned from the
Morton node index, which is deterministic in stream order"): the id is R-7's preorder index,
computed from the Morton-ordered stream. It is not a value derived from the cell key, which is
not unique for packed leaves and run sub-hierarchy nodes (v6.1 N4).

## P-A5. Slots for packed leaves and run sub-hierarchy nodes (v6.1 E48, N4)

- A **packed leaf** (consecutive leaf octants of one parent) takes the slot of its first octant;
  its row box lies inside the parent cell but may exceed that octant's cell (I1 reads rows,
  A-I1-3).
- A **degenerate cell** (key depth 21, still over `L` or `B_tile`) becomes an 8-ary run
  sub-hierarchy with synthetic slots 0–7, counted in `h`. Its nodes' identity is
  `(degenerate cell, run path)`, which the topology stream carries.
- P §4.4's duplicate-slot and slot ≥ branching-factor errors stay; the mesh producer never
  reaches them.

## P-A6. The 32-level bound is checked early by the producer (v6.1 stage 1b)

P §4.4 keeps `L > 32` as a typed writer error, but for the mesh producer it would come after the
whole LOD. The producer refuses a hierarchy of height > 32 with a typed Unsupported at stage 1b,
after pass 5a and before any LOD work, together with the exact U2 member-ceiling check. P's
writer error stays as a guard.

## P-A7. Evidence and acceptance (light process; ML3 part A)

- **Byte identity for the other producers:** point, vector (all modes) and legacy-mesh archives
  from the perf-lane inputs are byte-identical before and after ML3's writer change.
- **P1–P12 on mesh output** at CI size: `validate` (within today's caps) plus the writer's own
  checks, with packed leaves and run sub-hierarchy nodes.
- **P5 at scale** with a streaming expansion oracle (not the in-memory `expand_tileset`), on the
  ≥ 2^22-tile structural archive (slices ML3, RC16).
- **New P8 evidence:** per-document value nodes, bytes and JSON depth over the ≥ 2^22-tile
  archive and the mesh perf-lane forests (maximum and distribution).
- **Equivalence across chunking:** identical bytes at two run sizes `R`, two fan-ins `F` and
  `--jobs` 1, 2 and 8.
- **Cesium and request counts for mesh output** (the P §5.2 method) on Floreat and the scale
  synthetic, recorded as the mesh baseline.
- **Memory:** VmHWM of the writer flat from 2^16 to 2^22 tiles (perf lane).
- **Negative tests:** a topology/record mismatch is InvalidState; a unit over budget is the typed
  error; a hierarchy of height 33 is refused at stage 1b (P-A6).

## Settled items (coordinator, 2026-10-12)

- O-P-1: origin in the root transform, δ ≡ 0 (P-A1), reconciled with v6.1 above.
- O-P-2: bottom-up packing for mesh output only; the other producers byte-identical (P-A3).
- O-P-3: preorder names, reconciled with v6.1 §9.1 (P-A4).
- O-P-4: the hierarchy is final at pass 5a (v6.1 E49, N5).
