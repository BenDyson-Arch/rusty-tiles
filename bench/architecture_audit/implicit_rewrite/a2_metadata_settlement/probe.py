#!/usr/bin/env python3
"""Independent finite metadata/provenance models and pinned fixture inspection.

No converter/validator/build is invoked. This is not geometry, codec or viewer
acceptance. Argument: new evidence directory, portable-final2, native-final2.
"""
import copy
import hashlib
import json
import posixpath
from pathlib import Path
import re
import struct
import sys
from urllib.parse import unquote
import zipfile

SOURCE_MANIFEST = "a2-source-tileset.json"
SOURCE_REPORT = "a2-source-conversion.json"
GENERATED_CLASS = "a2Tile"
CONTROL_NAMES = {"tileset.json", "conversion.json", "@3dtilesIndex1@"}
KNOWN_PAYLOAD_EXTENSIONS = {"EXT_structural_metadata", "EXT_mesh_features", "EXT_meshopt_compression",
                            "KHR_materials_unlit", "KHR_mesh_quantization", "EXT_primitive_restart",
                            }


def sha(blob):
    return hashlib.sha256(blob).hexdigest()


class Refusal(Exception):
    def __init__(self, kind, reason):
        self.kind, self.reason = kind, reason
        super().__init__(reason)


def unsupported(reason):
    raise Refusal("unsupported", reason)


def invalid(reason):
    raise Refusal("invalid_input", reason)


def canonical(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()


def resolve_uri(base, uri):
    if not isinstance(uri, str) or not uri or any(c in uri for c in "\\:#?"):
        unsupported("URI outside archive-local finite profile")
    # Model only ordinary one-decode safe spellings; capture lane owns full URI
    # syntax/percent/separator/portable-name admission.
    decoded = unquote(uri)
    name = posixpath.normpath(posixpath.join(posixpath.dirname(base), decoded))
    if name.startswith("../") or name.startswith("/") or name == "..":
        unsupported("URI outside archive")
    return name


def inventory(names):
    files, parents = set(), set()
    for name in names:
        ancestors = {"/".join(name.split("/")[:i]) for i in range(1, len(name.split("/")))}
        if name in files or name in parents or ancestors & files:
            unsupported("fixed provenance/generated name collision")
        files.add(name)
        parents.update(ancestors)
    return files


def extensions(value):
    names = set()
    if isinstance(value, dict):
        if isinstance(value.get("extensions"), dict):
            names.update(value["extensions"])
        for key, item in value.items():
            if key != "extras":  # extras are opaque application JSON, not extensions.
                names.update(extensions(item))
    elif isinstance(value, list):
        for item in value:
            names.update(extensions(item))
    return names


def glb_document(blob):
    if blob[:4] == b"b3dm":
        if len(blob) < 28:
            invalid("truncated b3dm")
        version, length, fj, fb, bj, bb = struct.unpack_from("<6I", blob, 4)
        if version != 1 or length != len(blob):
            invalid("b3dm envelope")
        ft = json.loads(blob[28:28 + fj])
        if ft != {"BATCH_LENGTH": 0} or fb or bj or bb:
            unsupported("unproved b3dm feature/batch/RTC profile")
        blob = blob[28 + fj:]
    if len(blob) < 20 or blob[:4] != b"glTF":
        unsupported("unproved payload format")
    if struct.unpack_from("<II", blob, 4) != (2, len(blob)):
        invalid("GLB envelope")
    size, kind = struct.unpack_from("<II", blob, 12)
    if kind != 0x4e4f534a or size + 20 > len(blob):
        invalid("GLB JSON envelope")
    return json.loads(blob[20:20 + size])


def admit_manifest(manifest, names):
    if manifest.get("asset", {}).get("version") not in ("1.0", "1.1"):
        unsupported("unproved declaration")
    for key in ("schemaUri", "metadata", "groups", "statistics"):
        if key in manifest:
            unsupported("unproved tileset field: " + key)
    if manifest.get("extensions"):
        unsupported("unproved tileset extensions")
    allowed_extensions = {"3DTILES_content_gltf_vector"}
    if set(manifest.get("extensionsUsed", [])) - allowed_extensions:
        unsupported("unproved tileset extension declaration")
    if set(manifest.get("extensionsRequired", [])) - set(manifest.get("extensionsUsed", [])):
        invalid("required extension not declared used")
    if GENERATED_CLASS in manifest.get("schema", {}).get("classes", {}):
        unsupported("fixed generated schema class collision")
    rows, edges = [], []

    def visit(node, inherited, path):
        mode = node.get("refine", inherited)
        if mode is None:
            invalid("root refine is required")
        if mode not in ("REPLACE", "ADD"):
            invalid("malformed refine")
        if mode == "ADD":
            unsupported("effective ADD is outside replacement profile")
        for key in ("metadata", "viewerRequestVolume", "implicitTiling"):
            if key in node:
                unsupported("unproved tile field: " + key)
        if node.get("extensions"):
            unsupported("unproved tile extension")
        if "content" in node and "contents" in node:
            invalid("both content and contents")
        headers = node.get("contents", [node["content"]] if "content" in node else [])
        normalized = []
        for header in headers:
            for key in ("metadata", "boundingVolume", "group"):
                if key in header:
                    unsupported("unproved content field: " + key)
            if "url" in header and "uri" in header:
                invalid("ambiguous content reference")
            uri = header.get("uri", header.get("url"))
            name = resolve_uri("tileset.json", uri)
            if name in CONTROL_NAMES:
                unsupported("incoming live reference to replaced control")
            if name not in names:
                invalid("missing payload")
            if name.endswith(".json"):
                unsupported("external source tileset")
            ext = header.get("extensions", {})
            if set(ext) - allowed_extensions:
                unsupported("unproved content extension")
            if "3DTILES_content_gltf_vector" in ext and ext["3DTILES_content_gltf_vector"] != {"vector": True}:
                unsupported("unproved vector content extension value")
            retained = copy.deepcopy(header)
            retained.pop("url", None)
            retained["uri"] = uri
            normalized.append(retained)
            edges.append(("tileset.json", name, "content"))
        rows.append(dict(source_node_id=len(rows), source_path=path, effective_refine=mode,
                         source_extras_present="extras" in node,
                         source_extras_json=canonical(node.get("extras")).decode() if "extras" in node else "null",
                         content_headers=normalized))
        for ordinal, child in enumerate(node.get("children", [])):
            visit(child, mode, [*path, ordinal])
    visit(manifest["root"], None, [])
    return rows, edges


def prepare(files):
    """Finite metadata/name/provenance model only; no geometry plan is emitted."""
    manifest_raw = files["tileset.json"]
    manifest = json.loads(manifest_raw)
    rows, edges = admit_manifest(manifest, set(files))
    payload_facts = {}
    for _, name, _ in list(edges):
        if name in payload_facts:
            continue
        document = glb_document(files[name])
        unknown = extensions(document) - KNOWN_PAYLOAD_EXTENSIONS
        if unknown:
            unsupported("unproved payload spatial/resource extension: " + ",".join(sorted(unknown)))
        resources = []
        for item in [*document.get("buffers", []), *document.get("images", [])]:
            if "uri" in item:
                resources.append(item["uri"])
        metadata = document.get("extensions", {}).get("EXT_structural_metadata", {})
        if "schema" in metadata and "schemaUri" in metadata:
            invalid("ambiguous payload schema")
        if "schemaUri" in metadata:
            resources.append(metadata["schemaUri"])
        for uri in resources:
            target = resolve_uri(name, uri)
            if target in CONTROL_NAMES:
                unsupported("incoming payload resource reference to replaced control")
            if target not in files:
                invalid("missing known resource")
            edges.append((name, target, "known-payload-resource"))
        payload_facts[name] = dict(sha256=sha(files[name]), extensions=sorted(extensions(document)),
                                   inline_schema="schema" in metadata, external_schema=metadata.get("schemaUri"),
                                   extras_present="extras" in document)
    report_raw = files.get("conversion.json")
    report = json.loads(report_raw) if report_raw is not None else None
    if isinstance(report, dict) and "geometryReports" in report:
        target = resolve_uri("conversion.json", report["geometryReports"])
        if target in CONTROL_NAMES:
            unsupported("source report refers to replaced control")
        if target not in files:
            invalid("missing required source geometry report")
        edges.append(("conversion.json", target, "application-geometryReports"))
    raw_sources = {SOURCE_MANIFEST: manifest_raw}
    if report_raw is not None:
        raw_sources[SOURCE_REPORT] = report_raw
    # Known outgoing snapshot URI bases are root-level, unchanged. Unknown extras
    # are opaque values and supply no closed-world URI or standard semantics claim.
    for old, target, role in edges:
        if old in ("tileset.json", "conversion.json"):
            new = SOURCE_MANIFEST if old == "tileset.json" else SOURCE_REPORT
            assert resolve_uri(new, posixpath.relpath(target, posixpath.dirname(old) or ".")) == target
    names = [n for n in files if n not in CONTROL_NAMES]
    inventory([*names, *raw_sources, "tileset.json", "conversion.json", "@3dtilesIndex1@"])
    live_asset = copy.deepcopy(manifest["asset"])
    live_asset["version"] = "1.1"
    marker = live_asset.get("extras", {}).get("vectorBuildStateSha256") if isinstance(live_asset.get("extras"), dict) else None
    vector_binding = None
    if marker is not None:
        raw_state = files.get("vector-build.json")
        if raw_state is None:
            invalid("missing referenced vector state")
        state = json.loads(raw_state)
        # This checksum is one observed association, not reuse or geometry proof.
        vector_binding = dict(state_sha256=sha(raw_state), marker_matches_raw_state=marker == sha(raw_state),
                              declared_source_manifest_sha256=state.get("manifestSha256"),
                              live_reuse="unsupported-historical-state-only")
        # Exact compact current-producer bytes let us remove the one marker
        # without introducing a different floating-point canonical serializer.
        pattern = rb'"asset":\{"extras":\{"vectorBuildStateSha256":"[a-f0-9]{64}"\},"version":"1\.1"\}'
        stripped, count = re.subn(pattern, b'"asset":{"version":"1.1"}', manifest_raw, count=1)
        vector_binding["source_manifest_binding"] = ("matches" if sha(stripped) == state.get("manifestSha256") else "does-not-match") if count else "unverified-byte-recipe"
        del live_asset["extras"]["vectorBuildStateSha256"]
        if not live_asset["extras"]:
            del live_asset["extras"]
    return dict(profile="a2-metadata-provenance-initial-v1", rows=rows, edges=edges,
                payloads=payload_facts, raw_sources={n:dict(bytes=len(b),sha256=sha(b)) for n,b in raw_sources.items()},
                live_asset=live_asset, source_vector_binding=vector_binding,
                source_report_present=report_raw is not None,
                application_extras_policy="opaque-value mapping, source snapshot URI base; no generic URI/standard semantic equivalence")


def authored():
    document = {"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],
                "extensions":{"EXT_structural_metadata":{"schema":{"classes":{"feature":{"properties":{"tag":{"type":"STRING"}}}}}}}}
    data = canonical(document)
    data += b" " * (-len(data) % 4)
    glb = b"glTF" + struct.pack("<II",2,20+len(data)) + struct.pack("<II",len(data),0x4e4f534a) + data
    manifest = {"asset":{"version":"1.0"},"geometricError":10,"root":{"refine":"REPLACE","geometricError":10,"children":[{"content":{"url":"nested/payload.glb","extras":{"slot":"authored"}},"geometricError":0,"extras":{"tag":"retain","hint":"not a URI semantic"}}]}}
    return {"tileset.json":canonical(manifest), "nested/payload.glb":glb,
            "conversion.json":b'{ "geometryReports": "geometry-reports.jsonl", "opaque": true }\n',
            "geometry-reports.jsonl":b""}


def main():
    if len(sys.argv) != 4 or not __debug__:
        raise SystemExit("usage: python3 -B probe.py new-output portable-final2 native-final2 (no -O)")
    output=Path(sys.argv[1]).absolute()
    output.mkdir(parents=True,exist_ok=False)
    models=[]
    base=authored()
    positive=prepare(base)
    assert [r["effective_refine"] for r in positive["rows"]] == ["REPLACE","REPLACE"]
    assert positive["live_asset"]["version"] == "1.1"
    assert positive["raw_sources"][SOURCE_MANIFEST]["sha256"] == sha(base["tileset.json"])
    assert positive["raw_sources"][SOURCE_REPORT]["sha256"] == sha(base["conversion.json"])
    models.append(dict(case="inherited_replace_version_promotion_inline_payload_metadata_snapshot_bytes",outcome="admitted",result=positive))
    for label,value,present in [("absent",None,False),("null",None,True),("empty_object",{},True),("scalar",17,True),("array",[1,"two",None],True)]:
        files=copy.deepcopy(base); m=json.loads(files["tileset.json"])
        if present:m["root"]["extras"]=value
        files["tileset.json"]=canonical(m); admitted=prepare(files)
        row=admitted["rows"][0]
        assert row["source_extras_present"]==present and json.loads(row["source_extras_json"])==value
        models.append(dict(case="extras_"+label,outcome="admitted",present=present,value=value))
    files=copy.deepcopy(base); files.pop("conversion.json"); assert not prepare(files)["source_report_present"]
    models.append(dict(case="optional_source_report_absent",outcome="admitted"))
    files=copy.deepcopy(base); m=json.loads(files["tileset.json"])
    m["schema"]={"classes":{"rustyTile":{"properties":{"tag":{"type":"STRING"}}}}}
    files["tileset.json"]=canonical(m); prepare(files)
    models.append(dict(case="unused_inline_schema_old_class_name_retained",outcome="admitted"))
    # Metadata value/presence and exact provenance bytes are separate oracles.
    expected_row=positive["rows"][1]
    def extras_equivalent(row):
        return (row["source_extras_present"]==expected_row["source_extras_present"]
                and json.loads(row["source_extras_json"])==json.loads(expected_row["source_extras_json"]))
    for label,key,value in [("extras_present_collapsed","source_extras_present",False),
                            ("extras_value_changed","source_extras_json",'{"tag":"changed"}')]:
        wrong=copy.deepcopy(expected_row);wrong[key]=value;assert not extras_equivalent(wrong)
        models.append(dict(case=label,outcome="sensitive_control_detected"))
    assert sha(base["conversion.json"]+b" ") != positive["raw_sources"][SOURCE_REPORT]["sha256"]
    models.append(dict(case="same_report_value_different_raw_bytes",outcome="sensitive_control_detected"))
    files=copy.deepcopy(base);m=json.loads(files["tileset.json"]);m["asset"]["version"]="1.1"
    state=canonical({"manifestSha256":sha(canonical(m)),"version":1})
    m["asset"]["extras"]={"vectorBuildStateSha256":sha(state)}
    files["tileset.json"]=canonical(m);files["vector-build.json"]=state
    bound=prepare(files)
    assert bound["source_vector_binding"]["source_manifest_binding"]=="matches"
    assert "vectorBuildStateSha256" not in bound["live_asset"].get("extras",{})
    models.append(dict(case="source_vector_binding_historical_only",outcome="admitted",binding=bound["source_vector_binding"]))
    files["vector-build.json"]+=b" "
    mismatch=prepare(files)
    assert not mismatch["source_vector_binding"]["marker_matches_raw_state"]
    models.append(dict(case="changed_state_bytes_do_not_gain_verified_marker_status",outcome="sensitive_control_detected"))
    controls=[
        ("ADD_inherited",lambda m:m["root"].__setitem__("refine","ADD"),"unsupported"),
        ("ADD_child",lambda m:m["root"]["children"][0].__setitem__("refine","ADD"),"unsupported"),
        ("root_refine_absent",lambda m:m["root"].pop("refine"),"invalid_input"),
        ("unknown_refine",lambda m:m["root"].__setitem__("refine","TYPO"),"invalid_input"),
        ("tile_metadata",lambda m:m["root"].__setitem__("metadata",{"class":"authored","properties":{}}),"unsupported"),
        ("content_metadata",lambda m:m["root"]["children"][0]["content"].__setitem__("metadata",{"class":"authored","properties":{}}),"unsupported"),
        ("content_bounds",lambda m:m["root"]["children"][0]["content"].__setitem__("boundingVolume",{"sphere":[0,0,0,1]}),"unsupported"),
        ("content_group",lambda m:m["root"]["children"][0]["content"].__setitem__("group",0),"unsupported"),
        ("groups",lambda m:m.__setitem__("groups",[{"metadata":{"class":"authored"}}]),"unsupported"),
        ("tileset_metadata",lambda m:m.__setitem__("metadata",{"class":"authored"}),"unsupported"),
        ("external_tileset_schema",lambda m:m.__setitem__("schemaUri","schemas/tileset.json"),"unsupported"),
        ("unknown_spatial_tile_extension",lambda m:m["root"].__setitem__("extensions",{"VENDOR_bounds":{"sphere":[0,0,0,1]}}),"unsupported"),
        ("unknown_resource_content_extension",lambda m:m["root"]["children"][0]["content"].__setitem__("extensions",{"VENDOR_resource":{"uri":"nested/payload.glb"}}),"unsupported"),
        ("incoming_manifest_control",lambda m:m["root"]["children"][0]["content"].__setitem__("url","tileset.json"),"unsupported"),
        ("incoming_report_control",lambda m:m["root"]["children"][0]["content"].__setitem__("url","conversion.json"),"unsupported"),
        ("generated_class_collision",lambda m:m.__setitem__("schema",{"classes":{"a2Tile":{"properties":{}}}}),"unsupported"),
    ]
    for label,mutate,kind in controls:
        files=copy.deepcopy(base); m=json.loads(files["tileset.json"]); mutate(m); files["tileset.json"]=canonical(m)
        try:prepare(files)
        except Refusal as error:
            assert error.kind==kind;models.append(dict(case=label,outcome="refused",kind=kind,reason=error.reason))
        else:raise AssertionError(label)
    for label,names in [("source_snapshot_exact",[SOURCE_MANIFEST,SOURCE_MANIFEST]),("source_snapshot_ancestor",[SOURCE_REPORT+"/resource",SOURCE_REPORT])]:
        try:inventory(names)
        except Refusal as error:models.append(dict(case=label,outcome="refused",kind=error.kind))
        else:raise AssertionError(label)
    files=copy.deepcopy(base);files["conversion.json"]=canonical({"geometryReports":"conversion.json"})
    try:prepare(files)
    except Refusal as error:models.append(dict(case="incoming_report_control_edge",outcome="refused",kind=error.kind))
    else:raise AssertionError("incoming_report_control_edge")
    for target in sorted(CONTROL_NAMES):
        files=copy.deepcopy(base);document=glb_document(files["nested/payload.glb"])
        document["buffers"][0]["uri"]="../"+target
        encoded=canonical(document);encoded+=b" "*(-len(encoded)%4)
        files["nested/payload.glb"]=(b"glTF"+struct.pack("<II",2,20+len(encoded))
                                     +struct.pack("<II",len(encoded),0x4e4f534a)+encoded)
        try:prepare(files)
        except Refusal as error:
            assert error.kind=="unsupported"
            models.append(dict(case="incoming_payload_control_"+target,outcome="refused",kind=error.kind))
        else:raise AssertionError("incoming_payload_control_"+target)
    # Wrong-origin control: a subdirectory snapshot would change a known report edge.
    assert resolve_uri("conversion.json","geometry-reports.jsonl") != resolve_uri("provenance/conversion.json","geometry-reports.jsonl")
    models.append(dict(case="moving_provenance_to_subdirectory_changes_uri_base",outcome="sensitive_control_detected"))
    inspections=[]
    pins=[]
    for root_arg in sys.argv[2:]:
        root=Path(root_arg);receipt_raw=(root/"receipt.json").read_bytes();receipt=json.loads(receipt_raw)
        pins.append(dict(directory=str(root),receipt_sha256=sha(receipt_raw),execution_pins=receipt["pins"]))
        for case in receipt["cases"]:
            archive_raw=(root/(case["label"]+".3tz")).read_bytes()
            assert sha(archive_raw)==case["source_sha256"]
            with zipfile.ZipFile(root/(case["label"]+".3tz")) as archive:
                files={n:archive.read(n) for n in archive.namelist() if n!="@3dtilesIndex1@"}
            assert {n:sha(b) for n,b in files.items()}==case["source_members"]
            row=dict(mode="native" if "native" in root.name else "portable",label=case["label"],archive_sha256=sha(archive_raw),source_member_hashes=case["source_members"])
            try:row.update(outcome="initial_metadata_profile_admitted",inspection=prepare(files))
            except Refusal as error:row.update(outcome="initial_metadata_profile_refused",kind=error.kind,reason=error.reason)
            inspections.append(row)
    result=dict(evidence="independent metadata/provenance models plus read-only pinned fixture inspection; no producer/validator/build/viewer execution",driver_sha256=sha(Path(__file__).read_bytes()),models=models,inspections=inspections,originating_receipts=pins,production_executed=False)
    (output/"results.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(dict(models=len(models),inspections=len(inspections),admitted=sum(r["outcome"]=="initial_metadata_profile_admitted" for r in inspections),driver_sha256=result["driver_sha256"])))


if __name__=="__main__":main()
