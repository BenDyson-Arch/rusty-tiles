"""The oracle decision: precedence, domain predicate, precheck and streaming.

Outcome precedence (contract R8), as the oracle can observe it:
  1. payload records in traversal order: the first malformed scene/node record
     is InvalidInput(node) ("InvalidNode" here). Payload defects that C1 should
     reject make the oracle Undecided (it does not model C1).
  2. any exclusion reason anywhere -> NotInspected(all reasons, sorted).
  3. precheck in traversal order -> NotInspected(numericSpan | workLimit).
  4. streaming per reference / instance / accessor / vertex / level / role:
     the first proven escape -> Outside(facts).
  5. Contained.
"""

import hashlib
from fractions import Fraction

from . import spans as S
from .gltf import RESTART, Payload, PayloadError
from .numbers import OracleUndecided, frac_str
from .scene import NodeInvalid, QUAT_WINDOW, Walk, admit, quaternion_excess, select_scene, selected_nodes
from .tileset import Archive, Traversal
from .volumes import LevelTest

GLTF_ALLOWLIST = {
    "KHR_mesh_quantization",
    "EXT_meshopt_compression",
    "KHR_mesh_primitive_restart",
    "EXT_mesh_features",
    "EXT_structural_metadata",
    "EXT_mesh_polygon",
    "KHR_materials_unlit",
    "KHR_texture_basisu",
    "EXT_texture_webp",
}

REASONS = [
    "implicitTiling", "tilesetExtension", "tileMetadata", "upAxisOverride", "regionVolume",
    "rotatedBox", "rtcCenter", "gltfExtension", "sceneSelection", "skin", "morphTargets",
    "animation", "quaternionWindow", "numericSpan", "workLimit",
]

DEFAULT_LIMITS = {
    "containmentSpanBits": 4096,
    "containmentWorkUnits": 2 ** 28,  # coordinator cap decision (PR #157 review): was 2 ** 32
    "containmentBytesRead": 2 ** 31,
    "containmentDecodedBytes": 2 ** 31,
    "containmentNodeDepth": 64,
}

ORACLE_LIMITS = {
    "maxMemberBytes": 1 << 30,
    "maxTiles": 1_000_000,
    "maxVertexLevelTests": 200_000_000,
}

U = [[1, 0, 0], [0, 0, -1], [0, 1, 0]]


def ceil_div(a, b):
    return -(-a // b)


def _extension_names(obj, out, inside_extras=False):
    """Collect extension-object names, skipping opaque `extras` subtrees."""
    if isinstance(obj, dict):
        for key, value in obj.items():
            if key == "extras":
                continue
            if key == "extensions" and isinstance(value, dict):
                out.update(value.keys())
            _extension_names(value, out)
    elif isinstance(obj, list):
        for value in obj:
            _extension_names(value, out)


class UriFacts:
    """Phase-1 facts for one payload URI (computed once, never a verdict cache)."""

    def __init__(self, archive, member):
        self.member = member
        self.payload = Payload(archive, member)
        doc = self.payload.doc
        self.exclusions = set()
        self.c1_caveats = []
        admit(doc)  # raises NodeInvalid
        if self.payload.rtc_center:
            self.exclusions.add("rtcCenter")
        used = doc.get("extensionsUsed", [])
        required = doc.get("extensionsRequired", [])
        if not isinstance(used, list) or not isinstance(required, list):
            raise PayloadError("extensionsUsed/extensionsRequired is not an array in %s" % member)
        for name in used:
            if name not in GLTF_ALLOWLIST:
                self.exclusions.add("gltfExtension")
        for name in required:
            if name not in GLTF_ALLOWLIST:
                self.c1_caveats.append("%s: extensionsRequired %s is not allowlisted; C1 may "
                                       "refuse it (Unsupported) before C2" % (member, name))
        present = set()
        _extension_names(doc, present)
        for name in sorted(present - set(used)):
            self.c1_caveats.append("%s: extension object %s is not declared in extensionsUsed "
                                   "(C1 is expected to reject)" % (member, name))
        for mi, mesh in enumerate(doc.get("meshes", [])):
            for prim in (mesh or {}).get("primitives", []) or []:
                for sem in (prim or {}).get("attributes", {}) or {}:
                    if sem.startswith("JOINTS_") or sem.startswith("WEIGHTS_"):
                        self.c1_caveats.append("%s: mesh %d uses attribute %s; per the contract "
                                               "C1 rejects this semantic (Unsupported)" % (member, mi, sem))
        anims = doc.get("animations")
        if isinstance(anims, list) and anims:
            self.exclusions.add("animation")
        self.scene = select_scene(doc)
        self.instances = []
        self.walk = None
        if self.scene is None:
            self.exclusions.add("sceneSelection")
            self.node_units = 0
            self.max_depth = 0
        else:
            self._selected_exclusions(doc)
            self.walk = Walk(doc, self.scene)
            self.node_units = self.walk.node_units
            self.max_depth = self.walk.max_depth
        self._facts(doc)

    def _selected_exclusions(self, doc):
        nodes = doc.get("nodes", [])
        meshes = doc.get("meshes", [])
        for n, _, _ in selected_nodes(doc, self.scene):
            node = nodes[n]
            if "skin" in node:
                self.exclusions.add("skin")
            if "weights" in node:
                self.exclusions.add("morphTargets")
            if "rotation" in node and quaternion_excess(node["rotation"]) > QUAT_WINDOW:
                self.exclusions.add("quaternionWindow")
            if "mesh" in node:
                mesh = meshes[node["mesh"]]
                if not isinstance(mesh, dict):
                    raise PayloadError("mesh %d is not an object" % node["mesh"])
                if "weights" in mesh:
                    self.exclusions.add("morphTargets")
                for prim in mesh.get("primitives", []):
                    if isinstance(prim, dict) and "targets" in prim:
                        self.exclusions.add("morphTargets")

    def _facts(self, doc):
        p = self.payload
        self.E = sum(p.accessor_elements(i) for i in range(len(doc.get("accessors", []))))
        self.B = p.bytes_read
        self.Dec = p.decoded_bytes
        self.T = 0
        self.Idx = 0
        self.s_inst = 0
        self.s_pos = 0
        self.pair0 = None  # (P list, W) hull, or None without instances
        self.units_detail = []
        if self.walk is None:
            return
        meshes = doc.get("meshes", [])
        for inst in self.walk.instances:
            mesh = meshes[inst.mesh]
            prims = mesh.get("primitives", [])
            if not isinstance(prims, list):
                raise PayloadError("mesh %d primitives malformed" % inst.mesh)
            accessors = []  # distinct POSITION accessors, first-appearance order
            users = {}
            for pi, prim in enumerate(prims):
                if not isinstance(prim, dict):
                    raise PayloadError("primitive malformed")
                pos = (prim.get("attributes") or {}).get("POSITION")
                if "indices" in prim:
                    self.Idx += p.accessor_layout(prim["indices"])[3]
                elif pos is not None:
                    self.Idx += p.accessor_layout(pos)[3]
                if pos is None:
                    continue
                if pos not in users:
                    users[pos] = []
                    accessors.append(pos)
                users[pos].append(pi)
            inst.accessors = accessors
            inst.users = users
            self.s_inst = max(self.s_inst, inst.s_inst)
            for a in accessors:
                word, scale, k, count, _ = p.position_decoder(a)
                self.T += count
                tr = S.Tracker()
                P, W = S.level0(inst.G_pairs, S.WORD_PAIR[word], S.k_pair(k), tr)
                self.s_pos = max(self.s_pos, tr.max)
                if self.pair0 is None:
                    self.pair0 = (P, W)
                else:
                    self.pair0 = ([S.hull(x, y) for x, y in zip(self.pair0[0], P)],
                                  S.hull(self.pair0[1], W))

    def has_instances(self):
        return self.walk is not None and any(i.accessors for i in self.walk.instances)

    def summary(self):
        return {
            "member": self.member, "kind": self.payload.kind, "T": self.T,
            "Nodes": self.node_units, "Idx": self.Idx, "E": self.E, "Dec": self.Dec,
            "B": self.B, "maxNodeDepth": self.max_depth, "sInst": self.s_inst,
            "sPos": self.s_pos, "instances": len(self.walk.instances) if self.walk else 0,
            "exclusions": sorted(self.exclusions),
        }


class Oracle:
    def __init__(self, path, limits=None, oracle_limits=None):
        self.path = path
        self.limits = dict(DEFAULT_LIMITS)
        self.limits.update(limits or {})
        self.oracle_limits = dict(ORACLE_LIMITS)
        self.oracle_limits.update(oracle_limits or {})
        self.uri_facts = {}
        self.report = {"oracle": "rusty-tiles C2 O-explicit oracle", "archive": path,
                       "limits": self.limits, "oracleLimits": self.oracle_limits,
                       "c1Caveats": [], "caveats": [], "observations": []}
        with open(path, "rb") as fh:
            self.report["sha256"] = hashlib.sha256(fh.read()).hexdigest()

    # ------------------------------------------------------------ decision

    def run(self):
        try:
            outcome = self._decide()
        except OracleUndecided as err:
            outcome = {"outcome": "Undecided", "why": err.why}
        self.report.update(outcome)
        return self.report

    def _decide(self):
        self.archive = Archive(self.path, self.oracle_limits["maxMemberBytes"])
        trav = Traversal(self.archive, self.oracle_limits["maxTiles"]).run()
        self.traversal = trav
        self.report["caveats"] += trav.caveats
        self.report["references"] = len(trav.references)
        exclusions = set(trav.exclusions)
        sites = list(trav.exclusion_sites)
        for ref in trav.references:
            facts = self.uri_facts.get(ref.member)
            if facts is None:
                try:
                    facts = UriFacts(self.archive, ref.member)
                except NodeInvalid as err:
                    return {"outcome": "InvalidNode", "facts": dict(err.facts, why=err.why,
                                                                    member=ref.member, reference=ref.index)}
                self.uri_facts[ref.member] = facts
                self.report["c1Caveats"] += facts.c1_caveats
            for reason in facts.exclusions:
                exclusions.add(reason)
                sites.append({"reason": reason, "where": ref.member})
        self.report["payloads"] = [f.summary() for f in self.uri_facts.values()]
        if trav.invalid is not None:
            # Every reference before the defect was admitted; the defect is next in order.
            return {"outcome": "InvalidInput", "facts": {"why": trav.invalid.why,
                                                         "where": trav.invalid.where}}
        if exclusions:
            return {"outcome": "NotInspected", "reasons": sorted(exclusions),
                    "exclusionSites": sites[:200]}
        stop = self._precheck(trav.references)
        if stop is not None:
            return stop
        return self._stream(trav.references)

    def _precheck(self, references):
        totals = {"units": 0, "bytes": 0, "decoded": 0}
        per_ref = []
        self.report["precheck"] = {"totals": totals, "references": per_ref}
        cap = self.limits
        for ref in references:
            facts = self.uri_facts[ref.member]
            entry = {"reference": ref.index, "member": ref.member, "levels": len(ref.levels)}
            per_ref.append(entry)
            if facts.has_instances():
                level_spans = S.level_spans(facts.pair0, facts.s_pos, ref.levels)
                entry["sInst"] = facts.s_inst
                entry["levelSpans"] = level_spans
                if max([facts.s_inst] + level_spans) > cap["containmentSpanBits"]:
                    return {"outcome": "NotInspected", "reasons": ["numericSpan"],
                            "precheckStop": {"reference": ref.index, "member": ref.member,
                                             "reason": "numericSpan"}}
                vertex_units = facts.T * sum(ceil_div(s, 64) ** 2 for s in level_spans)
            else:
                vertex_units = 0
            units = vertex_units + facts.node_units + facts.Idx + facts.E
            entry.update({"units": units, "vertexUnits": vertex_units, "bytes": facts.B,
                          "decoded": facts.Dec, "maxNodeDepth": facts.max_depth})
            totals["units"] += units
            totals["bytes"] += facts.B
            totals["decoded"] += facts.Dec
            over = []
            if totals["units"] > cap["containmentWorkUnits"]:
                over.append("containmentWorkUnits")
            if totals["bytes"] > cap["containmentBytesRead"]:
                over.append("containmentBytesRead")
            if totals["decoded"] > cap["containmentDecodedBytes"]:
                over.append("containmentDecodedBytes")
            if facts.max_depth > cap["containmentNodeDepth"]:
                over.append("containmentNodeDepth")
            if over:
                return {"outcome": "NotInspected", "reasons": ["workLimit"],
                        "precheckStop": {"reference": ref.index, "member": ref.member,
                                         "reason": "workLimit", "exceeded": over}}
        totals["maxNodeDepth"] = max([f.max_depth for f in self.uri_facts.values()] or [0])
        return None

    # ------------------------------------------------------------ streaming

    def level_maps(self, inst, ref):
        """Exact affine maps (A, b) from node-input x to each level frame."""
        G = inst.G
        A = [[sum(U[r][m] * G[m][c] for m in range(3)) for c in range(3)] for r in range(3)]
        b = [sum(U[r][m] * G[m][3] for m in range(3)) for r in range(3)]
        maps = []
        for index, level in enumerate(ref.levels):
            if index and level.to_parent is not None:
                T = level.to_parent
                A = [[sum(T[r][m] * A[m][c] for m in range(3)) for c in range(3)] for r in range(3)]
                b = [sum(T[r][m] * b[m] for m in range(3)) + T[r][3] for r in range(3)]
            maps.append((A, b))
        return maps

    def drawn(self, payload, inst, accessor, count):
        mesh = payload.doc["meshes"][inst.mesh]
        marks = set()
        for pi in inst.users[accessor]:
            prim = mesh["primitives"][pi]
            if "indices" in prim:
                values, size = payload.index_values(prim["indices"])
                chunk = set(values)
                chunk.discard(RESTART[size])
                if chunk and max(chunk) >= count:
                    raise OracleUndecided("index %d >= POSITION count %d in %s (C1 rejects)"
                                          % (max(chunk), count, payload.member))
                marks |= chunk
            else:
                marks.update(range(count))
        return sorted(marks)

    def _stream(self, references):
        budget = self.oracle_limits["maxVertexLevelTests"]
        used = 0
        outside = None
        for ref in references:
            facts = self.uri_facts[ref.member]
            if not facts.has_instances():
                continue
            payload = facts.payload
            for inst in facts.walk.instances:
                maps = self.level_maps(inst, ref)
                for accessor in inst.accessors:
                    word, scale, k, count, decode = payload.position_decoder(accessor)
                    tests = []
                    for index, level in enumerate(ref.levels):
                        A, b = maps[index]
                        tests.append([(lv, LevelTest(lv.volume, A, b, scale))
                                      for lv in level.volumes_in_order()])
                    vertices = self.drawn(payload, inst, accessor, count)
                    used += len(vertices) * len(ref.levels)
                    if used > budget:
                        raise OracleUndecided("oracle vertex-level test budget exhausted")
                    values = decode(vertices)
                    for vi in vertices:
                        v = values[vi]
                        for level_tests in tests:
                            for lv, test in level_tests:
                                if not test.inside(v):
                                    outside = self._outside_facts(ref, inst, accessor, vi, v,
                                                                  scale, maps, lv, payload)
                                    break
                            if outside:
                                break
                        if outside:
                            break
                    if outside:
                        break
                if outside:
                    break
            if outside:
                break
        self._polygon_observation(references)
        if outside:
            return {"outcome": "Outside", "facts": outside}
        return {"outcome": "Contained"}

    def _outside_facts(self, ref, inst, accessor, vi, v, scale, maps, lv, payload):
        mesh = payload.doc["meshes"][inst.mesh]
        primitive = None
        for pi in inst.users[accessor]:
            prim = mesh["primitives"][pi]
            if "indices" not in prim:
                primitive = pi
                break
            values, size = payload.index_values(prim["indices"])
            if vi in set(values):
                primitive = pi
                break
        A, b = maps[lv.level]
        x = [c * scale for c in v]
        coord = [sum(A[r][c] * x[c] for c in range(3)) + b[r] for r in range(3)]
        return {
            "reference": ref.index, "uri": ref.uri, "member": ref.member,
            "contentAt": ref.where, "depth": ref.depth, "level": lv.level, "role": lv.role,
            "k": lv.level if lv.role in ("externalContent", "ancestor") else None,
            "volumeAt": lv.where, "node": inst.node, "mesh": inst.mesh,
            "primitive": primitive, "accessor": accessor, "vertex": vi,
            "coordinate": [frac_str(c) for c in coord],
            "coordinateApprox": [float(c) for c in coord],
        }

    def _polygon_observation(self, references):
        """Informative, non-gating: test every POSITION vertex of EXT_mesh_polygon primitives."""
        for ref in references:
            facts = self.uri_facts[ref.member]
            if not facts.has_instances():
                continue
            payload = facts.payload
            for inst in facts.walk.instances:
                mesh = payload.doc["meshes"][inst.mesh]
                for pi, prim in enumerate(mesh.get("primitives", [])):
                    if "EXT_mesh_polygon" not in (prim.get("extensions") or {}):
                        continue
                    accessor = prim["attributes"]["POSITION"]
                    word, scale, k, count, decode = payload.position_decoder(accessor)
                    maps = self.level_maps(inst, ref)
                    tests = [(lv, LevelTest(lv.volume, *maps[i], scale))
                             for i, level in enumerate(ref.levels) for lv in level.volumes_in_order()]
                    values = decode(range(count))
                    outside = [vi for vi in range(count)
                               if any(not t.inside(values[vi]) for _, t in tests)]
                    self.report["observations"].append({
                        "kind": "polygonAllVertices (informative, non-gating)",
                        "reference": ref.index, "member": ref.member, "node": inst.node,
                        "primitive": pi, "vertices": count, "outsideAnyVolume": len(outside),
                        "firstOutside": outside[:10]})

    # ------------------------------------------------------------ trace

    def trace(self):
        """Exact per-vertex frame values for witness comparison (no verdict)."""
        out = []
        for ref in self.traversal.references:
            facts = self.uri_facts.get(ref.member)
            if facts is None or facts.walk is None:
                continue
            payload = facts.payload
            for inst in facts.walk.instances:
                maps = self.level_maps(inst, ref)
                for accessor in inst.accessors:
                    word, scale, k, count, decode = payload.position_decoder(accessor)
                    vertices = self.drawn(payload, inst, accessor, count)
                    values = decode(vertices)
                    for vi in vertices:
                        x = [c * scale for c in values[vi]] + [Fraction(1)]
                        node_yp = [sum(inst.G[r][c] * x[c] for c in range(4)) for r in range(3)]
                        levels = []
                        for A, b in maps:
                            levels.append([sum(A[r][c] * x[c] for c in range(3)) + b[r]
                                           for r in range(3)])
                        out.append({"reference": ref.index, "node": inst.node,
                                    "accessor": accessor, "vertex": vi, "node_yp": node_yp,
                                    "levels": levels})
        return out
