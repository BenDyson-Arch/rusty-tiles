"""C2-WB unit arithmetic written a second time, from the contract text only.

This module does not import the oracle (`c2o`). The fixture generator states
each fixture's construction counts (T, Idx, E, Dec, B, J, Res, Prim, node
visits, per-level operations and, where derivable by hand, the span classes),
and this module turns them into the C2-WB total. The generator then requires
the oracle's total to be identical.

C2-WB 3.1 (frozen text), q_l = ceil(s_l/64)^2:

  V(r)     = sum_l ( [l = 0] max(20, q_l) + [l >= 1, child has transform] max(30, q_l)
                     + sum_{boxes at l} max(25, q_l) + sum_{spheres at l} max(32, q_l) )
  units(r) = T(u) V(r) + Nodes(u) + Idx(u) + E(u) + Dec(u) + B(u)
             + 16 J(u) + 160 Res(u) + 4 Prim(u) + 1152 + 8 (L_r + 1)

C2-WB 3.2: Nodes(u) = sum over node visits of 1 (skipped) or max(256, 128 ceil(s_step/64)^2).
"""

import json

BASE, STEP, BOX, SPHERE = 20, 30, 25, 32
REFERENCE_FIXED, PER_LEVEL = 1152, 8
PER_JSON_NODE, PER_RESOURCE, PER_PRIMITIVE = 16, 160, 4
NODE_FLOOR, NODE_WEIGHT = 256, 128


def q_of_span(s):
    return (-(-s // 64)) ** 2


def level_units(op, q):
    """op = (base, step, boxes, spheres) for one level; q = ceil(s/64)^2 there."""
    base, step, boxes, spheres = op
    return (base * max(BASE, q) + step * max(STEP, q) + boxes * max(BOX, q)
            + spheres * max(SPHERE, q))


def V(ops, qs):
    if len(ops) != len(qs):
        raise ValueError("ops and q lists differ in length")
    return sum(level_units(op, q) for op, q in zip(ops, qs))


def node_units(skipped, step_spans):
    """step_spans: {s_step: visits} for the non-skipped visits."""
    return skipped + sum(n * max(NODE_FLOOR, NODE_WEIGHT * q_of_span(s)) for s, n in step_spans.items())


def reference_units(u, levels):
    return (u["Dec"] + u["B"] + PER_JSON_NODE * u["J"] + PER_RESOURCE * u["Res"]
            + PER_PRIMITIVE * u["Prim"] + REFERENCE_FIXED + PER_LEVEL * levels)


def units_of_reference(u, ops, qs):
    vertex = u["T"] * V(ops, qs) if u["T"] else 0
    return vertex + u["Nodes"] + u["Idx"] + u["E"] + reference_units(u, len(ops))


def value_nodes(value):
    """Root plus every scalar/container value; object member names are not value nodes."""
    count, stack = 0, [value]
    while stack:
        v = stack.pop()
        count += 1
        if isinstance(v, dict):
            stack.extend(v.values())
        elif isinstance(v, list):
            stack.extend(v)
    return count


TYPE_SIZE = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}
TRS = ("matrix", "translation", "rotation", "scale")


def gltf_counts(doc):
    """Construction counts of a glTF document, by a direct walk of the selected scene:
    T, Idx, E, Prim, skipped and non-skipped node visits, maximum node depth."""
    acc = doc.get("accessors", [])
    E = sum(a["count"] * TYPE_SIZE[a["type"]] for a in acc)
    scene = doc["scene"] if "scene" in doc else 0
    nodes, meshes = doc.get("nodes", []), doc.get("meshes", [])
    T = Idx = Prim = skipped = steps = depth = 0
    stack = [(n, 1) for n in reversed(doc["scenes"][scene].get("nodes", []))]
    while stack:
        n, d = stack.pop()
        node = nodes[n]
        depth = max(depth, d)
        if any(k in node for k in TRS):
            steps += 1
        else:
            skipped += 1
        if "mesh" in node:
            prims = meshes[node["mesh"]].get("primitives", [])
            Prim += len(prims)
            distinct = []
            for p in prims:
                pos = p.get("attributes", {}).get("POSITION")
                Idx += acc[p["indices"]]["count"] if "indices" in p else (acc[pos]["count"] if pos is not None else 0)
                if pos is not None and pos not in distinct:
                    distinct.append(pos)
            T += sum(acc[a]["count"] for a in distinct)
        stack.extend((c, d + 1) for c in reversed(node.get("children", [])))
    return {"T": T, "Idx": Idx, "E": E, "Prim": Prim, "skipped": skipped, "steps": steps, "depth": depth}


def json_of(data):
    return json.loads(bytes(data).decode("utf-8"))
