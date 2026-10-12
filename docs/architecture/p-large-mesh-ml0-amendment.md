# P implicit layout: large-mesh tiler amendment (ML0, P-A1 to P-A7)

Status: **DRAFT, light process** (one fresh nonauthor review; USER process decision 2026-10-12).
Written by the ML0 author; not frozen. Items marked **[B2]**, **[B3]** or **[B4]** depend on the
design author's v5.1 fixes for those blockers of the v5 precode delta review and must be
re-checked against v5.1 before this note is frozen. Nothing here records acceptance.

This note amends [p-impl-layout-contract.md](p-impl-layout-contract.md) (frozen v3 plus the
delta-review amendments; sha256 `80acd233…0151`), which stays byte-identical. Where they disagree
on an item below, this note governs. ML3 part A (`implicit/tileset.rs`) implements it.

**Base:** `origin/develop` `e29cd19d2f236dc688e91a80a45170d4a573e20b`. **Design:** large-mesh v5
§9.1, §2.4, §2.6 (`design.md` `70a1eeec…afd1`); review `698b3b61…c4ef` (B2, B3, B4, E1).

## Unchanged

The layout (skeleton, implicit roots with one subtree, explicit leaves, link tiles, explicit
external documents); `S` = 4; `DOC_NODES` = 16,384, `DOC_BYTES` = 2 MiB, `DOC_LEVELS` = 10;
the `tileset.json` reserve of 64 nodes and 4 KiB (D-3); invariants P1–P12; the rebase and its
inverse (D-4); the failure list of §4.4; the I1 interface (§4.6); reuse (§4.7).

---

## P-A1. The large-mesh producer joins the domain

Add to §4.3 "Producers": **`mesh-local-to-3tz` (F1d3), OCTREE**, with:

- one content slot (`.glb`) on every tile, leaves and proxies alike;
- at most 8 children with distinct slots: the child's octant, or the rules of P-A5;
- **no child transforms.** Every tile is in the root tile's frame. The root tile's `transform`
  carries the rigid placement (F1c1 `--anchor` frame) composed with a translation by the global
  lattice origin. So δ(t) = 0 for every tile and every implicit payload is a byte copy (D-4: a zero
  δ is a byte copy for any suffix). This keeps the shared-lattice decode exact across tiles (a
  per-tile f64 rebase would round each tile's translation differently) and removes C2 rebase drift
  ([C2 amendment](c2-large-mesh-ml0-amendment.md) A-C2-2 item 4). The design's `ε_rebase` term
  (§3.1) is then 0 (O-P-1).

## P-A2. Streaming input in two phases

`write_forest` today takes the finished hierarchy H in memory. For the large-mesh producer H has
up to about 1.6×10^8 tiles (U2), so the writer takes two streams instead. Both list the nodes of H
in **post-order with children in ascending slot order**; the producer obtains this order by a
k-way merge of its per-depth, Morton-ordered spills (one cursor per depth, at most 32).

1. **Topology stream** (after the hierarchy is final, before the first payload is named): per
   node, its depth, its slot and whether it is a leaf. From it the writer computes, in one pass
   with a stack of at most 32 entries:
   - `h(n)` (P §4.1), and so every node's class (skeleton, explicit leaf, implicit root,
     implicit);
   - the preorder index of every implicit root (P-A4);
   - every implicit tile's root and local coordinates `(level, x, y, z)`, and so the **final
     member name** of every payload.

   It returns the name map as a spill keyed by stream position. The producer then writes each
   payload into the archive under its final name as soon as it is assembled (pass 7), in any
   order. There is no second payload copy.
2. **Record stream** (after pass 7): per node, `B(t)`, `e(t)`, `refine`, extras and its content
   `(member, bytes)`, in the same order. The writer emits subtree members and documents from it.

**Requirement [B2][B3][B4].** The topology must be final before phase 1. If v5.1 lets any later
pass change the hierarchy (for example byte-budget leaf cuts decided after the texture plan),
names can only be assigned after that pass, and payloads assembled before it must be spilled and
copied once more. The stage-2 scratch preflight must then count that copy. The design text today
says both "leaves are exact from the pass-0b histogram" and "stage 2 rechecks after the
byte-budget cuts"; v5.1 must say which.

**Bounded state.** Phase 1: the stack (32 entries) plus the name spill. Phase 2: an implicit
root's subtree is a contiguous run of the post-order stream ending at the root, so at most one
subtree is open, at most 585 records (OCTREE, `S` = 4); plus one document accumulator per path
level (P-A3). `M_layout ≤ D_open × DOC_BYTES ≤ 32 × 2 MiB` plus one subtree buffer.

**Disagreement** between the two streams (a node count, slot or leaf flag differs) is an internal
error (`InvalidState`), never a verdict about the input.

## P-A3. Bottom-up document packing (replaces breadth-first, for every producer)

P §4.1 packs documents breadth-first from the root, which needs the whole skeleton in memory.
The amended rule decides each cut when a skeleton child completes, with one open accumulator per
path level:

1. The accumulator of skeleton tile `t` holds `t`'s unit (its explicit leaves and implicit-root
   tiles, which are always kept with `t`) and the fragments of its accepted skeleton children.
2. When skeleton child `c` of `t` completes with fragment `F(c)`, it is **accepted** into `t`'s
   accumulator if the accumulator plus `F(c)` plus a **reserve** stays within `DOC_NODES` value
   nodes and `DOC_BYTES` serialized bytes, and `F(c)` spans at most `DOC_LEVELS − 1` skeleton
   levels. The reserve is an upper bound on `t`'s own tile object plus one link tile for each of
   `t`'s skeleton children not yet completed; the topology stream gives that count, and the
   writer fixes the bounds as profile constants from the widest f64 serialization.
3. Otherwise `c` is **cut**: `F(c)` is written as document `tileset-d{m}.json` with root `c`
   (without its transform), `m` counting from 1 in cut order, and a link tile for `c` (P §4.1) is
   appended to `t`'s accumulator.
4. The root's accumulator is `tileset.json`, with the D-3 reserve added to its reserve.
5. A single unit larger than the budget is the existing typed writer error.

P7 still holds: the bytes are a function of H and the profile constants, and also independent of
the stream chunking, `R`, `F` and `--jobs`. P8 holds by construction (every accept is checked).
P5 and P6 are unchanged: only *where* a cut falls changes.

**This changes the output of every producer** (point, vector and legacy mesh share
`write_forest`). It is a reuse-identity break for vector (`implicit*` source, recorded in ML3)
and needs the request-count and Cesium evidence of P §5.2–§5.3 re-run, because the coordinator's
v2 condition (request counts at or below today's) was shown for breadth-first packing (O-P-2).

## P-A4. Implicit-root names stay preorder indices

P §4.1 R-7 names root `r` by its preorder index `n` in H. It is computable in the post-order
stream: `n = c(r) + depth(r)`, where `c(r)` is the number of nodes the stream emitted before the
first node of `r`'s subtree. The design's "preorder ids from the Morton node index" (§9.1) is not
adopted: under octant packing and degenerate run sub-levels, a cell no longer identifies a node
uniquely [B2][B3] (O-P-3).

## P-A5. Slots for packed leaves and degenerate cells [B2][B3]

Provisional, pending v5.1:

- a **packed leaf** (several leaf octants of one parent merged) takes the slot of its first
  octant; its row box may exceed that octant's cell (I1 reads rows, A-I1-3);
- a **degenerate cell** (at the maximum key depth, still above `L`) is split into an 8-ary run
  sub-hierarchy with synthetic slots 0–7, counted in `h`;
- P §4.4's duplicate-slot and slot ≥ branching-factor errors stay; the mesh producer must never
  reach them.

## P-A6. The 32-level bound is checked early by the producer [B3]

P §4.4 keeps `L > 32` as a typed writer error, but for the mesh producer it would arrive after the
whole LOD. The producer must refuse a hierarchy of height > 32 with a typed Unsupported at the
same point as the U2 member-ceiling check (preflight stage 1), using the key depth that v5.1
states. P's writer error stays as a guard.

## P-A7. Evidence and acceptance (light process; ML3 part A)

- **P1–P12 on producer output** at CI size: `validate` (within today's caps) plus the writer's
  own checks; packed leaves and degenerate run sub-levels included once v5.1 defines them.
- **P5 at scale** with a streaming expansion oracle (not the in-memory `expand_tileset`), as
  slices ML3 (RC16) states, on the ≥ 2^22-tile structural archive.
- **New P8 evidence:** per-document value nodes, bytes and JSON depth over the ≥ 2^22-tile
  archive and the perf-lane forests (maximum and distribution).
- **Equivalence across chunking:** identical bytes at two run sizes `R`, two fan-ins `F` and
  `--jobs` 1, 2 and 8.
- **Re-run of P §5.2–§5.3** (Cesium fixtures and real cases, perf lane ABBA) after the packing
  change, for point and vector as well as mesh. Request counts must stay at or below today's on
  the P §5.2 cases.
- **Memory:** VmHWM of the writer flat from 2^16 to 2^22 tiles (perf lane).
- **Negative tests:** a topology/record mismatch is InvalidState; a unit over budget is the typed
  error; a hierarchy of height 33 is refused at preflight stage 1 (P-A6).

## Open questions (design author unless marked)

- **O-P-1.** Confirm δ ≡ 0 for the mesh producer (root transform carries the placement and the
  lattice origin; no child transforms). Design §3.1 still carries an `ε_rebase` term, which
  suggests nonzero δ was assumed. Nonzero δ would also need outward-rounded rows (A-C2-2) and an
  argument that per-tile rebased translations keep shared vertices bit-identical.
- **O-P-2 (coordinator).** Accept that bottom-up packing changes point and vector output too, and
  re-run the P §5.2–§5.3 evidence, or keep breadth-first for in-memory producers. The latter is a
  data-dependent second layout, which P §4.1 rules out.
- **O-P-3.** Confirm preorder naming (P-A4) instead of Morton-derived names.
- **O-P-4 [B4].** When is the hierarchy final (P-A2 requirement)?
