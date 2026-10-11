"""I1 preparation probe: decode implicit producer outputs from .3tz archives.

Stdlib only; run with `python3 -I`. Reads archives given as arguments and writes
one JSON report to the path given by --out. It imports nothing from the
repository and does not read src/. This is a fact-finding probe for contract
preparation, not the independent acceptance oracle (that is O-implicit).
"""
import argparse
import hashlib
import json
import struct
import sys
import zipfile
from fractions import Fraction as F


def popcount_bits(bits):
    return sum(1 for b in bits if b)


def morton(coords, scheme):
    x, y = coords[0], coords[1]
    z = coords[2] if scheme == "OCTREE" else 0
    out, bit = 0, 0
    lanes = [x, y, z] if scheme == "OCTREE" else [x, y]
    for i in range(32):
        for lane in lanes:
            out |= ((lane >> i) & 1) << bit
            bit += 1
    return out


def level_offset(level, n):
    return (n ** level - 1) // (n - 1)


class Subtree:
    def __init__(self, data, levels, n, content_count, read_member, name):
        self.facts = {"member": name, "bytes": len(data)}
        if len(data) < 24:
            raise ValueError("short subtree")
        magic, version, jlen, blen = struct.unpack("<4sIQQ", data[:24])
        self.facts.update(magic=magic.decode("latin1"), version=version,
                          jsonLength=jlen, binLength=blen,
                          jsonAligned=jlen % 8 == 0, binAligned=blen % 8 == 0,
                          totalMatches=24 + jlen + blen == len(data))
        js = data[24:24 + jlen]
        self.facts["jsonPaddingIsSpaces"] = js.rstrip(b" ") + b" " * (len(js) - len(js.rstrip(b" "))) == js
        doc = json.loads(js.decode("utf-8"))
        self.doc = doc
        binary = data[24 + jlen:24 + jlen + blen]
        buffers = doc.get("buffers", [])
        self.facts["buffers"] = [{k: v for k, v in b.items()} for b in buffers]
        blobs = []
        for i, b in enumerate(buffers):
            if "uri" in b:
                blobs.append(read_member(b["uri"]))
            else:
                if i != 0:
                    raise ValueError("internal buffer not first")
                blobs.append(binary)
        self.facts["binPaddingZero"] = (not buffers) or all(v == 0 for v in binary[buffers[0]["byteLength"]:]) if buffers and "uri" not in buffers[0] else None
        views = doc.get("bufferViews", [])
        self.facts["bufferViews"] = views
        self.facts["viewsAligned8"] = all(v.get("byteOffset", 0) % 8 == 0 for v in views)
        self.facts["viewsHaveByteOffset"] = all("byteOffset" in v for v in views)

        def view(i):
            v = views[i]
            blob = blobs[v["buffer"]]
            o = v["byteOffset"]
            return blob[o:o + v["byteLength"]]

        self.view = view
        nt = level_offset(levels, n)
        nc = n ** levels

        def avail(obj, length, label):
            rec = {"label": label}
            if "constant" in obj:
                rec["encoding"] = "constant"
                rec["constant"] = obj["constant"]
                bits = [obj["constant"] == 1] * length
            else:
                rec["encoding"] = "bitstream"
                raw = view(obj["bitstream"])
                rec["byteLength"] = len(raw)
                rec["expectedBytes"] = (length + 7) // 8
                bits = [(raw[i // 8] >> (i % 8)) & 1 == 1 for i in range(length)]
                trailing = [(raw[i // 8] >> (i % 8)) & 1 for i in range(length, len(raw) * 8)]
                rec["trailingZero"] = not any(trailing)
                # MSB-first reading, for a sensitivity observation only.
                msb = [(raw[i // 8] >> (7 - i % 8)) & 1 == 1 for i in range(length)]
                rec["msbFirstWouldDiffer"] = msb != bits
            rec["availableCountDeclared"] = obj.get("availableCount")
            rec["popcount"] = popcount_bits(bits)
            self.avail_records.append(rec)
            return bits

        self.avail_records = []
        self.tiles = avail(doc["tileAvailability"], nt, "tile")
        self.children = avail(doc["childSubtreeAvailability"], nc, "childSubtree")
        self.contents = [avail(c, nt, "content%d" % i) for i, c in enumerate(doc.get("contentAvailability", []))]
        self.facts["contentLayers"] = len(self.contents)
        self.facts["contentLayersMatchTemplates"] = len(self.contents) == content_count
        self.facts["availability"] = self.avail_records
        self.facts["tileMetadata"] = doc.get("tileMetadata")
        self.facts["contentMetadata"] = doc.get("contentMetadata")
        self.facts["subtreeMetadata"] = "subtreeMetadata" in doc
        self.facts["propertyTables"] = [
            {"class": t.get("class"), "count": t.get("count"),
             "properties": {k: sorted(v.keys()) for k, v in t.get("properties", {}).items()}}
            for t in doc.get("propertyTables", [])]
        self.facts["topLevelKeys"] = sorted(doc.keys())
        # Structural invariants (spec): parent availability, content => tile.
        bad_parent = 0
        for lvl in range(1, levels):
            for m in range(n ** lvl):
                if self.tiles[level_offset(lvl, n) + m] and not self.tiles[level_offset(lvl - 1, n) + m // n]:
                    bad_parent += 1
        self.facts["availableWithUnavailableParent"] = bad_parent
        self.facts["contentWithoutTile"] = sum(1 for layer in self.contents for i, b in enumerate(layer) if b and not self.tiles[i])
        last = level_offset(levels - 1, n)
        self.facts["childSubtreeUnderUnavailableTile"] = sum(
            1 for m, b in enumerate(self.children) if b and not self.tiles[last + m // n])


def box_of(volume):
    if "box" in volume:
        return [F(v) for v in volume["box"]]
    return None


def derived_box(root, level, coords, scheme):
    c = root[:3]
    axes = [root[3:6], root[6:9], root[9:12]]
    w = F(2) ** level
    out_c = list(c)
    out_axes = [list(a) for a in axes]
    count = 3 if scheme == "OCTREE" else 2
    for a in range(count):
        k = (2 * coords[a] + 1) / w - 1
        for lane in range(3):
            out_c[lane] += axes[a][lane] * k
            out_axes[a][lane] = axes[a][lane] / w
    return out_c + out_axes[0] + out_axes[1] + out_axes[2]


def box_inside(inner, outer):
    """Exact: is each corner of inner inside outer? Outer must be a diagonal box."""
    oc = outer[:3]
    oa = [outer[3:6], outer[6:9], outer[9:12]]
    for a in oa:
        if sum(1 for v in a if v != 0) > 1:
            return None
    half = [F(0)] * 3
    for a in oa:
        for j in range(3):
            if a[j] != 0:
                half[j] = abs(a[j])
    ic = inner[:3]
    ia = [inner[3:6], inner[6:9], inner[9:12]]
    for mask in range(8):
        p = [ic[j] + sum((ia[i][j] if mask >> i & 1 else -ia[i][j]) for i in range(3)) for j in range(3)]
        if any(abs(p[j] - oc[j]) > half[j] for j in range(3)):
            return False
    return True


def fill(template, level, x, y, z):
    return (template.replace("{level}", str(level)).replace("{x}", str(x))
            .replace("{y}", str(y)).replace("{z}", str(z)))


def resolve(base, uri):
    parts = base.split("/")[:-1]
    for seg in uri.split("/"):
        if seg in ("", "."):
            continue
        if seg == "..":
            parts.pop()
        else:
            parts.append(seg)
    return "/".join(parts)


def probe_archive(path):
    z = zipfile.ZipFile(path)
    names = set(z.namelist())
    report = {"archive": path, "sha256": hashlib.sha256(open(path, "rb").read()).hexdigest(),
              "members": len(names), "documents": [], "subtrees": [], "warnings": []}

    def read(name):
        return z.read(name)

    conv = json.loads(read("conversion.json")) if "conversion.json" in names else None
    if conv is not None:
        report["conversion"] = {k: conv.get(k) for k in ("operation", "encoder", "tiling", "maxPositionRoundingMetres") if k in conv}
    totals = {"availableTiles": 0, "contentTiles": 0, "rowEqualsDerived": 0, "rowInsideDerived": 0,
              "rowOutsideDerived": 0, "rowNotComparable": 0, "noRow": 0, "jsonContentRefs": 0,
              "jsonContentAtLevel0": 0, "errorEqualsHalving": 0, "errorDiffersHalving": 0,
              "missingContentMembers": 0, "rankRowsChecked": 0}
    visited_docs = []

    def walk_doc(name, depth, link=None):
        if name in visited_docs:
            report["warnings"].append("document revisited: " + name)
            return
        visited_docs.append(name)
        doc = json.loads(read(name))
        root = doc["root"]
        it = root.get("implicitTiling")
        drec = {"name": name, "depth": depth, "asset": doc.get("asset"),
                "extensionsUsed": doc.get("extensionsUsed"), "rootKeys": sorted(root.keys()),
                "rootTransform": root.get("transform"), "rootRefine": root.get("refine"),
                "rootBoundingVolumeKind": sorted(root.get("boundingVolume", {}).keys()),
                "implicitTiling": it, "schemaClasses": None, "extrasKeys": sorted(doc.get("extras", {}).keys())}
        schema = doc.get("schema")
        if schema:
            drec["schemaClasses"] = {
                cname: {pname: {k: p.get(k) for k in ("type", "componentType", "array", "count", "semantic", "normalized", "offset", "scale", "noData", "default", "required") if k in p}
                        for pname, p in c.get("properties", {}).items()}
                for cname, c in schema.get("classes", {}).items()}
        if link is not None:
            drec["linkRowEqualsRootVolume"] = link == box_of(root["boundingVolume"])
        templates = root.get("contents") or ([root["content"]] if "content" in root else [])
        drec["contentTemplates"] = [t.get("uri") for t in templates]
        drec["contentTemplateKeys"] = [sorted(t.keys()) for t in templates]
        report["documents"].append(drec)
        if not it:
            drec["explicit"] = True
            return
        scheme = it["subdivisionScheme"]
        n = 8 if scheme == "OCTREE" else 4
        levels = it["subtreeLevels"]
        avail_levels = it["availableLevels"]
        rootbox = box_of(root["boundingVolume"])
        root_error = F(root["geometricError"])
        semantic_props = {}
        for cname, c in (schema or {}).get("classes", {}).items():
            for pname, p in c.get("properties", {}).items():
                if "semantic" in p:
                    semantic_props.setdefault(cname, {})[p["semantic"]] = (pname, p)
        stack = [(0, 0, 0, 0)]
        max_level = 0
        cache = {}
        while stack:
            sl, sx, sy, sz = stack.pop()
            sname = resolve(name, fill(it["subtrees"]["uri"], sl, sx, sy, sz))
            if sname not in names:
                report["warnings"].append("missing subtree " + sname)
                continue
            st = Subtree(read(sname), levels, n, len(templates), lambda u: read(resolve(sname, u)), sname)
            st.facts["globalRoot"] = [sl, sx, sy, sz]
            report["subtrees"].append(st.facts)
            # metadata rows
            rows = {}
            if st.doc.get("tileMetadata") is not None:
                table = st.doc["propertyTables"][st.doc["tileMetadata"]]
                cls = table["class"]
                sem = semantic_props.get(cls, {})
                bb = sem.get("TILE_BOUNDING_BOX")
                ge = sem.get("TILE_GEOMETRIC_ERROR")
                st.facts["semanticsInTable"] = sorted(sem.keys())
                avail_idx = [i for i, b in enumerate(st.tiles) if b]
                st.facts["rowsEqualAvailable"] = table["count"] == len(avail_idx)
                bbv = st.view(table["properties"][bb[0]]["values"]) if bb else None
                gev = st.view(table["properties"][ge[0]]["values"]) if ge else None
                for r, idx in enumerate(avail_idx):
                    box = [F(v) for v in struct.unpack_from("<12d", bbv, r * 96)] if bbv else None
                    err = F(struct.unpack_from("<d", gev, r * 8)[0]) if gev else None
                    rows[idx] = (box, err)
                    totals["rankRowsChecked"] += 1
            for lvl in range(levels):
                gl = sl + lvl
                for m in range(n ** lvl):
                    idx = level_offset(lvl, n) + m
                    if not st.tiles[idx]:
                        continue
                    # decode morton -> local coords
                    lx = ly = lz = 0
                    for i in range(lvl):
                        if scheme == "OCTREE":
                            lx |= ((m >> (3 * i)) & 1) << i
                            ly |= ((m >> (3 * i + 1)) & 1) << i
                            lz |= ((m >> (3 * i + 2)) & 1) << i
                        else:
                            lx |= ((m >> (2 * i)) & 1) << i
                            ly |= ((m >> (2 * i + 1)) & 1) << i
                    gx, gy, gz = (sx << lvl) | lx, (sy << lvl) | ly, (sz << lvl) | lz
                    totals["availableTiles"] += 1
                    max_level = max(max_level, gl)
                    if gl >= avail_levels:
                        report["warnings"].append("tile beyond availableLevels")
                    derived = derived_box(rootbox, gl, (gx, gy, gz), scheme)
                    row = rows.get(idx)
                    if row is None or row[0] is None:
                        totals["noRow"] += 1
                    else:
                        if row[0] == derived:
                            totals["rowEqualsDerived"] += 1
                        else:
                            inside = box_inside(row[0], derived)
                            if inside is None:
                                totals["rowNotComparable"] += 1
                            elif inside:
                                totals["rowInsideDerived"] += 1
                            else:
                                totals["rowOutsideDerived"] += 1
                        if row[1] is not None:
                            if row[1] == root_error / F(2) ** gl:
                                totals["errorEqualsHalving"] += 1
                            else:
                                totals["errorDiffersHalving"] += 1
                    any_content = False
                    for slot, layer in enumerate(st.contents):
                        if layer[idx]:
                            any_content = True
                            uri = resolve(name, fill(templates[slot]["uri"], gl, gx, gy, gz))
                            if uri not in names:
                                totals["missingContentMembers"] += 1
                            if uri.endswith(".json"):
                                totals["jsonContentRefs"] += 1
                                if gl == 0:
                                    totals["jsonContentAtLevel0"] += 1
                                walk_doc(uri, depth + 1, link=row[0] if row else None)
                    if any_content:
                        totals["contentTiles"] += 1
            for m, b in enumerate(st.children):
                if b:
                    cx = cy = cz = 0
                    for i in range(levels):
                        if scheme == "OCTREE":
                            cx |= ((m >> (3 * i)) & 1) << i
                            cy |= ((m >> (3 * i + 1)) & 1) << i
                            cz |= ((m >> (3 * i + 2)) & 1) << i
                        else:
                            cx |= ((m >> (2 * i)) & 1) << i
                            cy |= ((m >> (2 * i + 1)) & 1) << i
                    stack.append((sl + levels, (sx << levels) | cx, (sy << levels) | cy, (sz << levels) | cz))
        drec["maxAvailableLevelObserved"] = max_level
        drec["availableLevelsDeclared"] = avail_levels

    walk_doc("tileset.json", 0)
    report["totals"] = totals
    report["subtreeCount"] = len(report["subtrees"])
    report["documentCount"] = len(report["documents"])
    # Compact the subtree facts: keep distributions, not every record.
    sts = report.pop("subtrees")
    report["subtreeSummary"] = {
        "encodings": sorted({(a["label"].rstrip("0123456789"), a["encoding"], a.get("availableCountDeclared") is not None) for s in sts for a in s["availability"]}),
        "allViewsAligned8": all(s["viewsAligned8"] for s in sts),
        "allViewsHaveByteOffset": all(s["viewsHaveByteOffset"] for s in sts),
        "allTrailingZero": all(a.get("trailingZero", True) for s in sts for a in s["availability"]),
        "allCountsMatch": all(a["availableCountDeclared"] in (None, a["popcount"]) for s in sts for a in s["availability"]),
        "anyMsbFirstWouldDiffer": any(a.get("msbFirstWouldDiffer") for s in sts for a in s["availability"]),
        "bufferShapes": sorted({json.dumps(s["buffers"] and [sorted(b.keys()) for b in s["buffers"]]) for s in sts}),
        "propertyTables": sorted({json.dumps(s["propertyTables"], sort_keys=True) for s in sts}),
        "semanticsInTable": sorted({json.dumps(s.get("semanticsInTable")) for s in sts}),
        "rowsEqualAvailable": all(s.get("rowsEqualAvailable", True) for s in sts),
        "topLevelKeys": sorted({json.dumps(s["topLevelKeys"]) for s in sts}),
        "headerOk": all(s["magic"] == "subt" and s["version"] == 1 and s["jsonAligned"] and s["binAligned"] and s["totalMatches"] for s in sts),
        "jsonPaddingIsSpaces": all(s["jsonPaddingIsSpaces"] for s in sts),
        "binPaddingZero": sorted({str(s["binPaddingZero"]) for s in sts}),
        "structuralViolations": sum(s["availableWithUnavailableParent"] + s["contentWithoutTile"] + s["childSubtreeUnderUnavailableTile"] for s in sts),
        "maxSubtreeBytes": max(s["bytes"] for s in sts),
        "totalSubtreeBytes": sum(s["bytes"] for s in sts),
        "childSubtreeBitsSet": sum(a["popcount"] for s in sts for a in s["availability"] if a["label"] == "childSubtree"),
    }
    return report


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("archives", nargs="+")
    a = ap.parse_args()
    reports = []
    for p in a.archives:
        try:
            reports.append(probe_archive(p))
        except Exception as e:  # recorded, not hidden
            reports.append({"archive": p, "error": repr(e)})
    with open(a.out, "x") as f:
        json.dump({"probe": "i1-inventory", "python": sys.version, "reports": reports}, f, indent=1, sort_keys=True, default=str)


if __name__ == "__main__":
    main()
