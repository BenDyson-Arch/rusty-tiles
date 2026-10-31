"""Scene/node admission, scene selection and the selected-scene walk.

Admission (contract "glTF records") raises NodeInvalid for every payload, on
all scenes and all nodes. The walk computes, per instance (a selected node
with `mesh`), the exact node composite G = L_1 ... L_m with the normalized
rotation meaning R(q) = H(q)/|q|^2 (D2), and the span facts of the normative
definition.
"""

from fractions import Fraction

from . import spans as S
from .numbers import exact, is_index, is_number

QUAT_WINDOW = Fraction(1, 2 ** 18)

# C2-WB 3.2 (frozen text): a skipped node visit is 1 unit, any other visit
# max(NODE_FLOOR, 128 * ceil(s_step / 64)^2).
NODE_SKIPPED = 1
NODE_WEIGHT = 128
NODE_FLOOR = 256


def node_visit_units(s_step):
    """Units of one non-skipped node visit with step span `s_step` (C2-WB 3.2)."""
    return max(NODE_FLOOR, NODE_WEIGHT * (-(-s_step // 64)) ** 2)


class NodeInvalid(Exception):
    def __init__(self, why, **facts):
        super().__init__(why)
        self.why = why
        self.facts = facts


def _finite_numbers(value, n):
    return isinstance(value, list) and len(value) == n and all(is_number(v) for v in value)


def _index_list(value, limit):
    return isinstance(value, list) and all(is_index(v) and v < limit for v in value)


def admit(doc):
    """Check the consumed scene/node records. Returns nothing; raises NodeInvalid."""
    nodes = doc.get("nodes", [])
    scenes = doc.get("scenes")
    meshes = doc.get("meshes", [])
    if not isinstance(nodes, list) or not all(isinstance(n, dict) for n in nodes):
        raise NodeInvalid("nodes is not an array of objects")
    if not isinstance(meshes, list):
        raise NodeInvalid("meshes is not an array")
    if scenes is not None and (not isinstance(scenes, list) or not all(isinstance(s, dict) for s in scenes)):
        raise NodeInvalid("scenes is not an array of objects")
    if "scene" in doc:
        if not is_index(doc["scene"]) or scenes is None or doc["scene"] >= len(scenes):
            raise NodeInvalid("scene does not index into scenes")
    count = len(nodes)
    for si, scene in enumerate(scenes or []):
        roots = scene.get("nodes", [])
        if not _index_list(roots, count):
            raise NodeInvalid("scenes[%d].nodes has an invalid node index" % si, scene=si)
        if len(set(roots)) != len(roots):
            raise NodeInvalid("scenes[%d].nodes repeats a node" % si, scene=si)
    parent_count = [0] * count
    parent = [None] * count
    for ni, node in enumerate(nodes):
        children = node.get("children", [])
        if not _index_list(children, count):
            raise NodeInvalid("nodes[%d].children has an invalid node index" % ni, node=ni)
        for c in children:
            parent_count[c] += 1
            parent[c] = ni
        if "mesh" in node and not (is_index(node["mesh"]) and node["mesh"] < len(meshes)):
            raise NodeInvalid("nodes[%d].mesh is not a valid mesh index" % ni, node=ni)
        _admit_fields(ni, node)
    for ni in range(count):
        if parent_count[ni] > 1:
            raise NodeInvalid("node %d has more than one parent" % ni, node=ni)
    for si, scene in enumerate(scenes or []):
        for root in scene.get("nodes", []):
            if parent_count[root]:
                raise NodeInvalid("scene %d root node %d has a parent" % (si, root), node=root)
    # Each node has at most one parent, so a cycle is a closed parent chain.
    state = [0] * count  # 0 unknown, 1 on current chain, 2 reaches a parentless node
    for start in range(count):
        chain = []
        n = start
        while n is not None and state[n] == 0:
            state[n] = 1
            chain.append(n)
            n = parent[n]
        if n is not None and state[n] == 1:
            raise NodeInvalid("node graph has a cycle through node %d" % n, node=n)
        for m in chain:
            state[m] = 2


def _admit_fields(ni, node):
    has_trs = any(k in node for k in ("translation", "rotation", "scale"))
    if "matrix" in node:
        m = node["matrix"]
        if not _finite_numbers(m, 16):
            raise NodeInvalid("nodes[%d].matrix is not 16 finite numbers" % ni, node=ni)
        if not (m[3] == 0 and m[7] == 0 and m[11] == 0 and m[15] == 1):
            raise NodeInvalid("nodes[%d].matrix last row is not 0,0,0,1" % ni, node=ni)
        if has_trs:
            raise NodeInvalid("nodes[%d] has matrix together with TRS" % ni, node=ni)
    if "rotation" in node:
        q = node["rotation"]
        if not _finite_numbers(q, 4) or not all(-1 <= v <= 1 for v in q):
            raise NodeInvalid("nodes[%d].rotation is not 4 numbers in [-1, 1]" % ni, node=ni)
        if all(v == 0 for v in q):
            raise NodeInvalid("nodes[%d].rotation is zero" % ni, node=ni)
    for key in ("translation", "scale"):
        if key in node and not _finite_numbers(node[key], 3):
            raise NodeInvalid("nodes[%d].%s is not 3 finite numbers" % (ni, key), node=ni)


def select_scene(doc):
    """Index of the selected scene, or None for `sceneSelection`."""
    if "scene" in doc:
        return doc["scene"]
    scenes = doc.get("scenes")
    if isinstance(scenes, list) and len(scenes) == 1:
        return 0
    return None


def selected_nodes(doc, scene_index):
    """Root-first DFS of the selected scene: list of (node index, depth, path)."""
    nodes = doc.get("nodes", [])
    out = []
    roots = doc["scenes"][scene_index].get("nodes", [])
    stack = [(r, 1, (r,)) for r in reversed(roots)]
    while stack:
        n, depth, path = stack.pop()
        out.append((n, depth, path))
        for c in reversed(nodes[n].get("children", [])):
            stack.append((c, depth + 1, path + (c,)))
    return out


def is_skipped(node):
    return not any(k in node for k in ("matrix", "translation", "rotation", "scale"))


def quaternion_excess(q):
    """|sum q^2 - 1| exactly."""
    return abs(sum(exact(v) ** 2 for v in q) - 1)


# ---------------------------------------------------------------- exact matrices

def mat_identity():
    return [[Fraction(int(i == j)) for j in range(4)] for i in range(4)]


def mat_mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def column_major(values):
    """4x4 exact matrix from 16 column-major JSON numbers."""
    return [[exact(values[4 * c + r]) for c in range(4)] for r in range(4)]


def node_matrix(node):
    """Exact local matrix L = T * R(q) * S, R(q) = H(q)/|q|^2, or the column-major matrix."""
    if "matrix" in node:
        return column_major(node["matrix"])
    t = [exact(v) for v in node.get("translation", [0, 0, 0])]
    s = [exact(v) for v in node.get("scale", [1, 1, 1])]
    x, y, z, w = [exact(v) for v in node.get("rotation", [0, 0, 0, 1])]
    n = x * x + y * y + z * z + w * w
    H = [
        [w * w + x * x - y * y - z * z, 2 * (x * y - w * z), 2 * (x * z + w * y)],
        [2 * (x * y + w * z), w * w - x * x + y * y - z * z, 2 * (y * z - w * x)],
        [2 * (x * z - w * y), 2 * (y * z + w * x), w * w - x * x - y * y + z * z],
    ]
    L = mat_identity()
    for r in range(3):
        for c in range(3):
            L[r][c] = H[r][c] / n * s[c]
        L[r][3] = t[r]
    return L


# ---------------------------------------------------------------- span facts

def node_pairs(node):
    if "matrix" in node:
        return ("matrix", [S.pair_of(exact(v)) for v in node["matrix"]])
    t = [S.pair_of(exact(v)) for v in node.get("translation", [0, 0, 0])]
    s = [S.pair_of(exact(v)) for v in node.get("scale", [1, 1, 1])]
    q = [S.pair_of(exact(v)) for v in node.get("rotation", [0, 0, 0, 1])]
    return ("trs", (t, q, s))


class Instance:
    def __init__(self, node, mesh, depth, path, G_exact, G_pairs, s_inst):
        self.node = node
        self.mesh = mesh
        self.depth = depth
        self.path = path
        self.G = G_exact
        self.G_pairs = G_pairs
        self.s_inst = s_inst  # max s_step over this instance's path


class Walk:
    """Selected-scene walk with exact composites and span facts.

    Composites (exact and pair) are computed for every selected node so that
    Nodes(u) can charge every non-skipped node; sInst only covers instance paths
    (accepted reading v3.2 (9)). Node units follow C2-WB 3.2: 1 per skipped
    visit, otherwise max(256, 128 * ceil(s_step/64)^2)."""

    def __init__(self, doc, scene_index):
        nodes = doc.get("nodes", [])
        self.instances = []
        self.node_units = 0
        self.max_depth = 0
        self.node_visits = 0
        self.skipped_visits = 0
        self.step_spans = {}  # s_step -> number of non-skipped visits with that span
        state = {}  # node -> (G_exact, G_pairs, max s_step along path)
        for n, depth, path in selected_nodes(doc, scene_index):
            node = nodes[n]
            self.node_visits += 1
            self.max_depth = max(self.max_depth, depth)
            if len(path) > 1:
                G, Gp, s_path = state[path[-2]]
            else:
                G, Gp, s_path = mat_identity(), S.identity_g(), 0
            if is_skipped(node):
                self.node_units += NODE_SKIPPED
                self.skipped_visits += 1
            else:
                tr = S.Tracker()
                kind, data = node_pairs(node)
                if kind == "matrix":
                    L = S.matrix_l(data, tr)
                else:
                    L = S.trs_l(data[0], data[1], data[2], tr)
                Gp = S.compose(Gp, L, tr)
                G = mat_mul(G, node_matrix(node))
                s_path = max(s_path, tr.max)
                self.node_units += node_visit_units(tr.max)
                self.step_spans[tr.max] = self.step_spans.get(tr.max, 0) + 1
            state[n] = (G, Gp, s_path)
            if "mesh" in node:
                self.instances.append(Instance(n, node["mesh"], depth, path, G, Gp, s_path))
