"""C2 slice 3: decoded-content containment for implicit archives (O-implicit-C2).

Contract: C2 slice-3 delta, frozen v3.1 (sha256 CONTRACT_SHA256), sections 1-4 and 9.A.

This module composes the two committed independent oracles; it re-derives
neither of them.

- O-implicit (`implicit_read/oracle/i1_oracle.py`, `evaluate()`) supplies the
  visited implicit tiles of every implicit root, in I1 walk order, with their
  effective volumes and emitted contents.
- O-explicit (`c2o/`, as amended by C2-WB) supplies the explicit traversal,
  payload facts, spans, the precheck and exact streaming.

What it adds (contract 9.A):

- the level chain of every emitted content (section 4 table): the tile's own
  row, its implicit ancestors, I_0, R's JSON volume V_R, then R.transform and
  the explicit levels above R, with an external-root R's referencing content
  volume at level l+2;
- I5: a reference whose chain holds a rowless tile at level >= 1 is excluded
  with `derivedImplicitVolume` (chain-scoped, decided before the precheck);
- I6: template-header `metadata` is `tileMetadata` (O-explicit's content loop
  already sees template headers); subtree metadata tables never are;
- I7: implicit levels carry only their own volume, with no tile step;
- `implicitTiling` is never produced;
- section 3 Outside facts: `depth` = depth(R) + l (I1's depth), `k` = chain
  level index.

Outcome precedence (contract section 3): I1 first (a non-Accepted O-implicit
verdict passes through as the outcome), then O-explicit's phase 1 (payload
admission), exclusions, precheck, streaming.

Pure Python 3 standard library. It reads no `src/`, `tests/` or `bindings/`.
"""

import hashlib
import importlib.util
import os
import struct
import sys

from . import tileset as TS
from .check import DEFAULT_LIMITS, Oracle
from .numbers import OracleUndecided
from .volumes import Volume, parse_volume

CONTRACT_SHA256 = "8bc534290c5a90ace963e345336dea1e04c45fafb3c869672ee4af1ba03331d6"

HERE = os.path.dirname(os.path.abspath(__file__))
I1_ORACLE_PATH = os.path.normpath(os.path.join(HERE, "..", "..", "..", "implicit_read", "oracle", "i1_oracle.py"))

# The composed oracles, pinned (contract 9.A). `pins()` reports drift; the
# self-test fails on it. Mutant copies of c2o are expected to drift.
PINS = {
    "c2o/check.py": "e0209cb4db3dd09049e5ce8da10ecae11454213a2a31acc9eb310df10bbdf56f",
    "c2o/gltf.py": "00afbf3261eeabad9c3c00883dc7c21ad0dd6e05b7c8c8de934ad9caa9e825e2",
    "c2o/meshopt.py": "946cbe81bf3baedcd3c1adb6863d4fb6ff309e28defa53ad5f1b9dfa22ef21e6",
    "c2o/numbers.py": "d28df9e87fa957a1a755bdbe409a2cf1e2c4fe4f76a0cc30075ad759ff06b40c",
    "c2o/scene.py": "dd06ea0593244280cfc9a9930c8ed4aae0aa94fc830006d0c46aab014dc710c5",
    "c2o/spans.py": "dadbebe2fba7bb132b906c98557606141e16215fa3b8959fe5afc1c0c01c82bb",
    "c2o/tileset.py": "3460b08cf3cc3022bf05090770ed0b91e269a137899a55e37f7d21702021ae75",
    "c2o/volumes.py": "b9a7456247d0668aec40497e5e33b833e141035bcbfe8af22f9b9db6e528bfce",
    "implicit_read/oracle/i1_oracle.py": "62fdc706e40c0b7e2930c1c4c23a80cd7ac861610db3029036740ba81fe5c7c0",
}

# Slice 3 consumes C2-WB's frozen default cap (contract header: 2^29). O-explicit's
# DEFAULT_LIMITS carried 2^28 when O wrote this; R's integration fix set it to 2^29
# (and re-pinned c2o/check.py above). The explicit override is kept, now a no-op.
SLICE3_LIMITS = dict(DEFAULT_LIMITS)
SLICE3_LIMITS["containmentWorkUnits"] = 2 ** 29

# The #160 ValidationLimits defaults O passes to O-implicit (contract 9.A "Limits");
# O-implicit's own defaults are the pre-#160 values.
I1_DEFAULTS_160 = {
    "hierarchy_visits": 2 ** 18,
    "references": 2 ** 20,
    "member_bytes": 128 << 20,
    "document_decoded_bytes": 128 << 20,
    "document_items": 2 ** 17,
}

NO_ROOT = "archive has no implicit root"
DERIVED_REASON = "derivedImplicitVolume"


def _sha256(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def pins():
    """{file: [pinned, actual]} for every composed file that drifted (empty when all match)."""
    out = {}
    oracle_dir = os.path.dirname(HERE)
    for rel, want in PINS.items():
        path = I1_ORACLE_PATH if rel.startswith("implicit_read/") else os.path.join(oracle_dir, rel)
        got = _sha256(path) if os.path.exists(path) else None
        if got != want:
            out[rel] = [want, got]
    return out


_I1 = None


def i1_module():
    global _I1
    if _I1 is None:
        spec = importlib.util.spec_from_file_location("c2s3_i1_oracle", I1_ORACLE_PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _I1 = module
    return _I1


def i1_limits(overrides=None):
    lim = dict(I1_DEFAULTS_160)
    lim.update(overrides or {})
    return lim


class DerivedVolume(Volume):
    """A tile at level >= 1 without a TILE_BOUNDING_BOX row (I5): never evaluated."""

    def __init__(self, raw):
        super().__init__("derived", raw=raw)

    def exclusion(self):
        return DERIVED_REASON


def _row(text):
    return [struct.unpack(">d", bytes.fromhex(h))[0] for h in text.split(",")]


def effective_volume(rec, root_volume, where):
    """Section 4 step 3: Row -> exact box (FLOAT32 already widened by I1),
    DerivedRoot -> exactly V_R, Derived -> never evaluated."""
    vol = rec["volume"]
    if vol.startswith("row:"):
        values = _row(vol[4:])
        if len(values) != 12:
            raise OracleUndecided("I1 row at %s does not have 12 values" % where)
        return parse_volume({"box": values}, where)
    if vol == "derived-root":
        if rec["level"] != 0:
            raise OracleUndecided("derived-root volume at level %d (%s)" % (rec["level"], where))
        return root_volume
    if vol.startswith("derived:"):
        if rec["level"] == 0:
            raise OracleUndecided("derived cell at level 0 (%s)" % where)
        return DerivedVolume(vol)
    raise OracleUndecided("unknown I1 volume form %r at %s" % (vol[:40], where))


def split_roots(tiles):
    """I1's tile list split at each I_0 (parent -1): one group per implicit root, in order."""
    groups = []
    for rec in tiles:
        if rec["parent"] == -1:
            if rec["level"] != 0:
                raise OracleUndecided("I1 record with parent -1 at level %d" % rec["level"])
            groups.append([])
        elif not groups:
            raise OracleUndecided("I1 tile list does not start at an I_0")
        groups[-1].append(rec)
    return groups


class ImplicitTraversal(TS.Traversal):
    """O-explicit's traversal with every implicit root expanded in place (section 3
    item 4: R's position in the explicit pre-order; R has no explicit content or
    children, so nothing explicit interleaves)."""

    def __init__(self, archive, max_tiles, groups, roots):
        super().__init__(archive, max_tiles)
        self.groups = groups
        self.roots = roots
        self.roots_seen = 0
        self.implicit_references = 0
        self.implicit_tiles = 0
        self.max_chain = 0

    def _tile(self, root, member, root_where, stack, documents):
        # O-explicit's Traversal._tile (c2o/tileset.py, pinned), unchanged except the
        # implicit branch: no implicitTiling exclusion; the I1 walk runs in place.
        path = list(stack)
        work = [(root, root_where, len(stack))]
        while work:
            tile, where, depth = work.pop()
            del path[depth:]
            self.tiles += 1
            if self.tiles > self.max_tiles:
                raise OracleUndecided("tile count exceeds the oracle's tile limit")
            if not isinstance(tile, dict) or "boundingVolume" not in tile:
                raise OracleUndecided("tile without boundingVolume at %s" % where)
            self._check_extensions(tile, where)
            if isinstance(tile.get("boundingVolume"), dict):
                self._check_extensions(tile["boundingVolume"], where + ".boundingVolume")
            volume = TS.parse_volume(tile["boundingVolume"], where)
            if "metadata" in tile:
                self.exclude("tileMetadata", where + ".metadata")
            frame = TS._Frame(tile, where, volume, TS._transform(tile, where), None)
            frame.external_root = bool(stack) and depth == len(stack)
            path.append(frame)
            implicit = "implicitTiling" in tile
            for ci, content in enumerate(TS._contents(tile)):
                cwhere = "%s.content[%d]" % (where, ci)
                self._check_extensions(content, cwhere)
                if "metadata" in content:
                    # I6: a template header's `metadata` is copied to every emitted content.
                    self.exclude("tileMetadata", cwhere + ".metadata")
                cvol = None
                if "boundingVolume" in content:
                    if isinstance(content["boundingVolume"], dict):
                        self._check_extensions(content["boundingVolume"], cwhere + ".boundingVolume")
                    cvol = TS.parse_volume(content["boundingVolume"], cwhere)
                if implicit:
                    if cvol is not None:
                        raise OracleUndecided("template with content.boundingVolume at %s (I1 refuses)" % cwhere)
                    continue  # template URI, not a reference
                uri = content.get("uri")
                if not isinstance(uri, str):
                    raise OracleUndecided("content without uri at %s" % cwhere)
                target = TS.resolve(member, uri)
                if target.endswith(".json"):
                    if target in documents:
                        raise OracleUndecided("external tileset cycle at %s" % cwhere)
                    outer = list(path)
                    outer[-1] = TS._Frame(tile, where, volume, frame.transform,
                                          (cvol, cwhere) if cvol is not None else None)
                    outer[-1].external_root = frame.external_root
                    self._tileset(target, outer, documents + [target])
                else:
                    self._reference(target, uri, cwhere, path, cvol)
            if implicit:
                if tile.get("children"):
                    raise OracleUndecided("implicit root with children at %s (I1 refuses)" % where)
                self._walk_implicit(tile, member, where, path)
                continue
            children = tile.get("children", [])
            if isinstance(children, list):
                for ci in reversed(range(len(children))):
                    work.append((children[ci], "%s.children[%d]" % (where, ci), len(path)))

    def _walk_implicit(self, tile, member, where, path):
        """Section 4 steps 3-4: one frame per visited implicit tile, one reference
        per emitted content, in I1 walk order."""
        index = self.roots_seen
        self.roots_seen += 1
        if index >= len(self.groups):
            raise OracleUndecided("implicit root %d at %s has no O-implicit record" % (index, where))
        group = self.groups[index]
        root_frame = path[-1]
        depth_r = len(path) - 1
        first = group[0]
        # Root-order identity (9.A self-test 3): the k-th implicit root of the explicit
        # traversal is O-implicit's k-th root: same document, depth, scheme and subtree template.
        record = self.roots[index] if index < len(self.roots) else {}
        tiling = tile.get("implicitTiling") or {}
        subtrees = (tiling.get("subtrees") or {}).get("uri")
        same = (first["doc"] == member and first["depth"] == depth_r and record.get("doc") == member
                and record.get("scheme") == tiling.get("subdivisionScheme")
                and isinstance(subtrees, str) and record.get("subtrees") == TS.resolve(member, subtrees))
        if not same:
            raise OracleUndecided("root-order mismatch at %s: O-implicit root %d is %s depth %d"
                                  % (where, index, first["doc"], first["depth"]))
        stack = []
        for rec in group:
            level = rec["level"]
            if rec["parent"] != level - 1 or level > len(stack) or rec["depth"] != depth_r + level:
                raise OracleUndecided("I1 record out of walk order at %s level %d" % (where, level))
            del stack[level:]
            twhere = "%s.implicit[%d/%d/%d/%d]" % (where, level, rec["x"], rec["y"], rec["z"])
            vol = effective_volume(rec, root_frame.volume, twhere)
            if rec["volume"].startswith("derived:"):
                root_hex = rec["volume"][len("derived:"):].split(";")[0]
                root_box = (root_frame.volume.raw or {}).get("box") or []
                if _row(root_hex) != [float(v) for v in root_box]:
                    raise OracleUndecided("derived cell root box differs from V_R at %s" % twhere)
            frame = TS._Frame(None, twhere, vol, None, None)  # I7: no transform, no content volume
            stack.append(frame)
            self.implicit_tiles += 1
            self.tiles += 1
            if self.tiles > self.max_tiles:
                raise OracleUndecided("tile count exceeds the oracle's tile limit")
            chain = path + stack
            for slot, target in rec["contents"]:
                cwhere = "%s.content[%d]" % (twhere, slot)
                self._reference(target, target, cwhere, chain, None)
                ref = self.references[-1]
                ref.depth = rec["depth"]  # section 3: depth(R) + l, not the chain length - 1
                ref.implicit = {"root": where, "level": level, "x": rec["x"], "y": rec["y"], "z": rec["z"],
                                "slot": slot}
                self.implicit_references += 1
                self.max_chain = max(self.max_chain, len(ref.levels))


class ImplicitOracle(Oracle):
    """O-explicit's decision over the slice-3 traversal, after O-implicit."""

    def __init__(self, path, limits=None, oracle_limits=None, i1_limit_overrides=None, precheck_only=False,
                 detail=True):
        merged = dict(SLICE3_LIMITS)
        merged.update(limits or {})
        super().__init__(path, merged, oracle_limits, precheck_only=precheck_only, detail=detail)
        self.i1_limits = i1_limits(i1_limit_overrides)
        self.report["oracle"] = "rusty-tiles C2 O-implicit-C2 oracle (slice 3)"
        self.report["contract"] = CONTRACT_SHA256
        self.report["i1Limits"] = self.i1_limits
        drift = pins()
        if drift:
            self.report["pinDrift"] = drift

    def _decide(self):
        i1 = i1_module().evaluate(self.path, self.i1_limits)
        verdict = i1["verdict"]
        self.report["i1"] = {"verdict": verdict, "counts": i1["counts"], "tilesSha256": i1.get("tilesSha256")}
        if verdict["kind"] == "Accepted":
            groups = split_roots(i1["tiles"])
            roots = i1["roots"]
            if len(roots) != len(groups):
                raise OracleUndecided("O-implicit lists %d roots and %d I_0 records" % (len(roots), len(groups)))
        elif verdict["kind"] == "Undecided" and (verdict["reason"] or "").startswith(NO_ROOT):
            groups, roots = [], []  # explicit-only archive: slice 1 unchanged
        elif verdict["kind"] == "Undecided":
            return {"outcome": "Undecided", "why": "O-implicit: %s" % verdict["reason"]}
        else:
            # Outcome 1: an I1 outcome precedes everything C2 decides.
            return {"outcome": verdict["kind"], "phase": "I1",
                    "facts": {"why": verdict["reason"], "stage": verdict["stage"], "limit": verdict["limit"]}}
        self.archive = TS.Archive(self.path, self.oracle_limits["maxMemberBytes"])
        trav = ImplicitTraversal(self.archive, self.oracle_limits["maxTiles"], groups, roots).run()
        self.traversal = trav
        if trav.invalid is None and trav.roots_seen != len(groups):
            raise OracleUndecided("explicit traversal met %d implicit roots, O-implicit %d"
                                  % (trav.roots_seen, len(groups)))
        self.report["implicit"] = {"roots": len(groups), "implicitTiles": trav.implicit_tiles,
                                   "implicitReferences": trav.implicit_references,
                                   "maxChainLevels": trav.max_chain}
        return self._after_traversal(trav)

    def _after_traversal(self, trav):
        # O-explicit's Oracle._decide (c2o/check.py, pinned) from the traversal on.
        self.report["caveats"] += trav.caveats
        self.report["references"] = len(trav.references)
        exclusions = set(trav.exclusions)
        sites = list(trav.exclusion_sites)
        for ref in trav.references:
            facts = self.uri_facts.get(ref.member)
            if facts is None:
                try:
                    facts = _uri_facts(self.archive, ref.member)
                except _node_invalid() as err:
                    return {"outcome": "InvalidNode", "facts": dict(err.facts, why=err.why,
                                                                    member=ref.member, reference=ref.index)}
                self.uri_facts[ref.member] = facts
                self.report["c1Caveats"] += facts.c1_caveats
            for reason in facts.exclusions:
                exclusions.add(reason)
                sites.append({"reason": reason, "where": ref.member})
        self.report["payloads"] = [f.summary() for f in self.uri_facts.values()]
        if trav.invalid is not None:
            return {"outcome": "InvalidInput", "facts": {"why": trav.invalid.why, "where": trav.invalid.where}}
        if exclusions:
            return {"outcome": "NotInspected", "reasons": sorted(exclusions), "exclusionSites": sites[:200]}
        stop = self._precheck(trav.references)
        if stop is not None:
            return stop
        if self.precheck_only:
            return {"outcome": "PrecheckPassed"}
        return self._stream(trav.references)

    def _outside_facts(self, ref, inst, accessor, vi, v, scale, maps, lv, payload):
        facts = super()._outside_facts(ref, inst, accessor, vi, v, scale, maps, lv, payload)
        implicit = getattr(ref, "implicit", None)
        if implicit is not None:
            facts["implicit"] = implicit
        facts["volume"] = lv.role if lv.role in ("own", "content") else "%s:%d" % (lv.role, lv.level)
        return facts


def _uri_facts(archive, member):
    from .check import UriFacts
    return UriFacts(archive, member)


def _node_invalid():
    from .scene import NodeInvalid
    return NodeInvalid


def run(path, limits=None, oracle_limits=None, i1_limit_overrides=None, precheck_only=False, detail=True):
    return ImplicitOracle(path, limits, oracle_limits, i1_limit_overrides, precheck_only, detail).run()


if __name__ == "__main__":  # pragma: no cover
    sys.exit("use oracle/c2_oracle.py --implicit")
