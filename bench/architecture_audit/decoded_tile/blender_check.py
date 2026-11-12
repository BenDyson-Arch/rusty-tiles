"""Import decoded tiles with Blender's bundled glTF importer and report what survives.

Run headless inside official Blender with the wheel installed in PACKAGES:

    blender -b --factory-startup --python blender_check.py -- PACKAGES REPORT.json ARCHIVE[:URI] ...

Without `:URI`, the archive's root (coarse) tile and its largest leaf are checked. For
each tile, the plain GLB from `decode_tile(..., format="glb")` and the published
member are imported with default options. The report records, per tile:
- whether each import finished (the published member is expected to fail on KTX2);
- objects, polygons against the decoded triangle count, and placement: every
  polygon's world-space corners against `format="arrays"` positions (Blender's
  Z-up import frame is the arrays' tile frame);
- materials with a base-colour image node holding pixels, UV layers, colour
  attributes and custom attributes (name, domain, type);
- per-face feature IDs: Blender stores `_FEATURE_ID_n` on points and merges
  vertices that differ only in custom attributes, so the report counts faces whose
  three corner values agree and equal the decoded per-face ID.
"""
import json
from pathlib import Path
import sys
import time
import zipfile

argv = sys.argv[sys.argv.index("--") + 1:]
packages, report_path, targets = argv[0], Path(argv[1]), argv[2:]
sys.path.insert(0, packages)

import bpy  # noqa: E402
import numpy as np  # noqa: E402
import rusty_tiles  # noqa: E402


def import_glb(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    start = time.perf_counter()
    try:
        result = bpy.ops.import_scene.gltf(filepath=str(path))
        return {"result": sorted(result), "seconds": round(time.perf_counter() - start, 3)}
    except Exception as error:  # the published member: the importer's failure line
        lines = [line.strip() for line in str(error).splitlines() if "Error" in line]
        return {"result": ["FAILED"], "error": (lines[-1] if lines else str(error))[:300]}


def scene_facts(decoded):
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    facts = {"objects": len(meshes), "polygons": sum(len(o.data.polygons) for o in meshes)}
    facts["triangles"] = sum(len(p["indices"]) for p in decoded["primitives"])
    images, uv_layers, colors, attributes = set(), set(), set(), {}
    for o in meshes:
        uv_layers.update(layer.name for layer in o.data.uv_layers)
        colors.update(a.name for a in o.data.color_attributes)
        for a in o.data.attributes:
            if a.name.startswith("_"):
                attributes[a.name] = [a.domain, a.data_type]
        for slot in o.material_slots:
            if slot.material and slot.material.use_nodes:
                for node in slot.material.node_tree.nodes:
                    if node.type == "TEX_IMAGE" and node.image is not None and node.image.size[0] > 0:
                        images.add((node.image.name, tuple(node.image.size)))
    facts.update(materials=len(bpy.data.materials), textured_images=sorted(map(list, images)),
                 uv_layers=sorted(uv_layers), color_attributes=sorted(colors), custom_attributes=attributes)
    facts["pages"] = [list(page.shape[1::-1]) for page in decoded["pages"]]
    # Placement and per-face IDs, mesh by mesh (one Blender object per glTF node mesh).
    by_mesh = {}
    for p in decoded["primitives"]:
        by_mesh.setdefault((p["node"], p["mesh"]), []).append(p)
    position_error, faces, consistent, correct = 0.0, 0, {}, {}
    if len(meshes) == len(by_mesh):
        for o, (_, primitives) in zip(sorted(meshes, key=lambda o: o.name), sorted(by_mesh.items())):
            mesh = o.data
            world = np.array(o.matrix_world)
            co = np.array([v.co for v in mesh.vertices], dtype=np.float64)
            co = co @ world[:3, :3].T + world[:3, 3]
            corners = np.concatenate([p["positions"][p["indices"]] for p in primitives])
            loops = np.array([list(poly.vertices) for poly in mesh.polygons])
            if loops.shape != corners.shape[:2]:
                position_error = None
                break
            # The importer builds loops in index order, so corners compare directly.
            position_error = max(position_error or 0.0, float(np.abs(co[loops] - corners).max()))
            faces += len(loops)
            for n, ids in enumerate(primitives[0]["feature_ids"]):
                name = f"_FEATURE_ID_{n}"
                if name not in mesh.attributes:
                    continue
                values = np.array([d.value for d in mesh.attributes[name].data])
                expected = np.concatenate([p["feature_ids"][n]["values"][p["indices"][:, 0]] for p in primitives])
                at = values[loops]
                same = (at == at[:, :1]).all(axis=1)
                label = ids["label"]
                consistent[label] = consistent.get(label, 0) + int(same.sum())
                correct[label] = correct.get(label, 0) + int((same & (at[:, 0] == expected)).sum())
    facts.update(position_max_abs=position_error, faces_compared=faces,
                 feature_faces_consistent=consistent, feature_faces_correct=correct)
    return facts


report = {"blender": bpy.app.version_string, "build_hash": bpy.app.build_hash.decode(),
          "rusty_tiles": rusty_tiles.__version__, "tiles": []}
work = report_path.parent / "blender-glb"
work.mkdir(parents=True, exist_ok=True)
for target in targets:
    archive, _, uri = target.partition(":")
    tiles = rusty_tiles.list_tiles(archive)
    with zipfile.ZipFile(archive) as z:
        size = {info.filename: info.file_size for info in z.infolist()}
    leaves = [t["uri"] for t in tiles if t["role"] == "leaf"]
    picks = [uri] if uri else [tiles[0]["uri"], max(leaves, key=size.get)]
    for uri in picks:
        role = next(t["role"] for t in tiles if t["uri"] == uri)
        plain = work / f"{Path(archive).stem}-{uri.replace('/', '_')}"
        plain.write_bytes(rusty_tiles.decode_tile(archive, uri))
        decoded = rusty_tiles.decode_tile(archive, uri, format="arrays")
        entry = {"archive": Path(archive).name, "uri": uri, "role": role, "plain_bytes": plain.stat().st_size}
        entry["plain"] = import_glb(plain)
        if entry["plain"]["result"] == ["FINISHED"]:
            entry["plain"].update(scene_facts(decoded))
        published = work / ("published-" + plain.name)
        with zipfile.ZipFile(archive) as z:
            published.write_bytes(z.read(uri))
        entry["published"] = import_glb(published)
        report["tiles"].append(entry)
        print(json.dumps(entry), flush=True)
report_path.write_text(json.dumps(report, indent=1) + "\n")
