#!/usr/bin/env python3
"""Generate or check the standard build's GDAL-verified EPSG allow-list.

  generate.py write    regenerate src/crs/epsg_allowlist.tsv.gz and the reference fixture
  generate.py check    regenerate into a scratch directory; fail on any drift
  generate.py versions print the GDAL/PROJ/EPSG versions recorded in the committed table

Needs GDAL's Python bindings, the PROJ library they use and a Rust toolchain; see README.md.
Every code is sampled at an 18-point lattice and kept only when one of its generated
definitions, run through the crate's own resolver and proj4rs, lands within 1 mm in ECEF
of GDAL's only-best, no-ballpark, network-off result at every sample.
"""
import argparse
import ctypes
import ctypes.util
import difflib
import gzip
import json
import math
import os
import pathlib
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
TABLE = ROOT / "src/crs/epsg_allowlist.tsv.gz"
FIXTURE = ROOT / "tests/fixtures/crs_allowlist_reference.json"
FORMAT = "rusty-tiles EPSG allow-list, format 1"
TOLERANCE_M = 0.001
HEIGHTS = (0.0, 1000.0)
SAMPLES = 18
# Pipeline steps that change the datum. An operation using none of them, and no grid,
# is a null transformation (or a pure conversion): towgs84 zero reproduces it.
DATUM_STEPS = ("helmert", "hgridshift", "vgridshift", "deformation", "molodensky",
               "molobadekas", "geogoffset", "horner", "xyzgridshift", "tinshift")
# Codes with GDAL-computed ECEF reference fixtures for the crate's unit tests.
FIXTURE_CODES = (7856, 28356, 2193, 25832, 4283, 7844, 32756)
# PJ_TYPE values from proj.h.
PJ_CONVERSION, PJ_TRANSFORMATION, PJ_CONCATENATED = 21, 22, 23


# The caller's environment, for cargo/rustup, before PROJ is isolated.
CALLER_ENV = dict(os.environ)


def isolate_environment(scratch):
    """Offline PROJ with only its own database: no network, no user grids or caches."""
    os.environ["PROJ_NETWORK"] = "OFF"
    os.environ["HOME"] = os.environ["XDG_DATA_HOME"] = str(scratch)
    os.environ["PROJ_USER_WRITABLE_DIRECTORY"] = str(scratch)
    if "PROJ_DATA" not in os.environ:
        prefix = os.environ.get("CONDA_PREFIX")
        if not prefix:
            sys.exit("error: set PROJ_DATA or activate the conda environment (CONDA_PREFIX)")
        os.environ["PROJ_DATA"] = str(pathlib.Path(prefix) / "share/proj")
    data = pathlib.Path(os.environ["PROJ_DATA"])
    if not (data / "proj.db").is_file():
        sys.exit(f"error: no proj.db in PROJ_DATA={data}")
    grids = sorted(p.name for p in data.iterdir() if p.suffix.lower() in (".tif", ".tiff", ".gsb", ".gtx", ".byn"))
    if grids:
        # The table states what a grid-free PROJ does; installed grids change GDAL's answers.
        sys.exit(f"error: PROJ_DATA holds grid files ({', '.join(grids[:5])}...); use a grid-free PROJ_DATA")
    return data


class Proj:
    """The few PROJ C functions GDAL's Python bindings do not expose."""

    def __init__(self, library):
        self.lib = ctypes.CDLL(library)
        P, S, I, D = ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_double
        spec = {
            "proj_context_create": (P, []),
            "proj_log_level": (I, [P, I]),
            "proj_create": (P, [P, S]),
            "proj_destroy": (P, [P]),
            "proj_get_name": (S, [P]),
            "proj_get_type": (I, [P]),
            "proj_get_id_auth_name": (S, [P, I]),
            "proj_get_id_code": (S, [P, I]),
            "proj_as_proj_string": (S, [P, P, I, P]),
            "proj_create_operation_factory_context": (P, [P, S]),
            "proj_operation_factory_context_set_allow_ballpark_transformations": (None, [P, P, I]),
            "proj_operation_factory_context_set_grid_availability_use": (None, [P, P, I]),
            "proj_operation_factory_context_set_spatial_criterion": (None, [P, P, I]),
            "proj_operation_factory_context_destroy": (None, [P]),
            "proj_create_operations": (P, [P, P, P, P]),
            "proj_list_get_count": (I, [P]),
            "proj_list_get": (P, [P, P, I]),
            "proj_list_destroy": (None, [P]),
            "proj_coordoperation_get_grid_used_count": (I, [P, P]),
            "proj_coordoperation_get_accuracy": (D, [P, P]),
            "proj_coordoperation_get_towgs84_values": (I, [P, P, ctypes.POINTER(D), I, I]),
            "proj_concatoperation_get_step_count": (I, [P, P]),
            "proj_concatoperation_get_step": (P, [P, P, I]),
        }
        for name, (restype, argtypes) in spec.items():
            function = getattr(self.lib, name)
            function.restype, function.argtypes = restype, argtypes
        lib = self.lib
        self.ctx = lib.proj_context_create()
        lib.proj_log_level(self.ctx, 0)
        # GDAL's only-best operation ranking: ballpark off, every candidate listed whatever
        # its grids (a missing grid on the best operation makes GDAL refuse, not fall back).
        self.factory = lib.proj_create_operation_factory_context(self.ctx, None)
        lib.proj_operation_factory_context_set_allow_ballpark_transformations(self.ctx, self.factory, 0)
        lib.proj_operation_factory_context_set_grid_availability_use(self.ctx, self.factory, 2)
        lib.proj_operation_factory_context_set_spatial_criterion(self.ctx, self.factory, 1)
        self.wgs84 = lib.proj_create(self.ctx, b"EPSG:4326")

    def _towgs84(self, op):
        values = (ctypes.c_double * 7)()
        if self.lib.proj_coordoperation_get_towgs84_values(self.ctx, op, values, 7, 0):
            return [float(v) for v in values]
        return None

    def _identifier(self, obj):
        auth, code = self.lib.proj_get_id_auth_name(obj, 0), self.lib.proj_get_id_code(obj, 0)
        return f"{auth.decode()}:{code.decode()}" if auth and code else ""

    def operations(self, code):
        """Ranked operations from EPSG:code to WGS 84, each with what a definition needs."""
        lib, ctx = self.lib, self.ctx
        source = lib.proj_create(ctx, f"EPSG:{code}".encode())
        found = lib.proj_create_operations(ctx, source, self.wgs84, self.factory)
        result = []
        for index in range(lib.proj_list_get_count(found) if found else 0):
            op = lib.proj_list_get(ctx, found, index)
            pipeline = lib.proj_as_proj_string(ctx, op, 0, None)
            pipeline = pipeline.decode() if pipeline else None
            kind = lib.proj_get_type(op)
            steps = [op]
            if kind == PJ_CONCATENATED:
                steps = [lib.proj_concatoperation_get_step(ctx, op, k)
                         for k in range(max(0, lib.proj_concatoperation_get_step_count(ctx, op)))]
            transformations = [s for s in steps if lib.proj_get_type(s) == PJ_TRANSFORMATION]
            shift = self._towgs84(op)
            if shift is None and pipeline and "helmert" in pipeline:
                # A concatenated conversion + Helmert: take the single Helmert step's shift.
                shifts = [v for v in (self._towgs84(s) for s in steps) if v is not None]
                shift = shifts[0] if len(shifts) == 1 else None
            result.append({
                "name": lib.proj_get_name(op).decode(),
                "datum_operation": " + ".join(lib.proj_get_name(s).decode() for s in transformations),
                "datum_operation_id": " + ".join(self._identifier(s) for s in transformations),
                "accuracy": lib.proj_coordoperation_get_accuracy(ctx, op),
                "grids": lib.proj_coordoperation_get_grid_used_count(ctx, op),
                "pipeline": pipeline,
                "towgs84": shift,
            })
            if steps != [op]:
                for step in steps:
                    lib.proj_destroy(step)
            lib.proj_destroy(op)
        if found:
            lib.proj_list_destroy(found)
        lib.proj_destroy(source)
        return result


def candidate_shift(op):
    """The towgs84 a definition needs to reproduce this operation, or None if it cannot."""
    if op["grids"]:
        return None
    if op["towgs84"] is not None:
        return op["towgs84"]
    if op["pipeline"] and not any(step in op["pipeline"] for step in DATUM_STEPS):
        return [0.0] * 7
    return None


def gdal_modules():
    from osgeo import gdal, osr
    gdal.UseExceptions()
    osr.UseExceptions()
    gdal.PushErrorHandler("CPLQuietErrorHandler")
    return gdal, osr


def spatial_reference(osr, code):
    s = osr.SpatialReference()
    s.ImportFromEPSG(int(code))
    s.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    s.PromoteTo3D(None)
    return s


def strict_transform(osr, source, target):
    options = osr.CoordinateTransformationOptions()
    options.SetOnlyBest(True)
    options.SetBallparkAllowed(False)
    try:
        return osr.CreateCoordinateTransformation(source, target, options)
    except RuntimeError:
        return None


def transform_point(transform, point):
    if transform is None or point is None:
        return None
    try:
        result = list(transform.TransformPoint(*point))
    except RuntimeError:
        return None
    return result if all(math.isfinite(v) for v in result) else None


def exported(function, *args):
    try:
        return function(*args) or None
    except RuntimeError:
        return None


def inventory(osr, proj_db):
    """Active EPSG projected and geographic 2D CRSs with their merged areas of use."""
    db = sqlite3.connect(f"file:{proj_db}?mode=ro", uri=True)
    active = {int(code) for (code,) in db.execute(
        "select code from crs_view where auth_name='EPSG' and type in ('projected','geographic 2D') and deprecated=0")}
    epsg = db.execute("select value from metadata where key='EPSG.VERSION'").fetchone()[0]
    db.close()
    areas = {}
    for info in osr.GetCRSInfoListFromDatabase("EPSG"):
        if info.code.isdigit() and int(info.code) in active:
            areas.setdefault(int(info.code), []).append(
                (info.west_lon_degree, info.south_lat_degree, info.east_lon_degree, info.north_lat_degree))
    return epsg, {code: areas[code] for code in sorted(areas)}


def lattice(boxes):
    """3x3 longitude/latitude lattice over the area of use, Greenwich degrees."""
    if len(boxes) == 1:
        west, south, east, north = boxes[0]
        east = east + 360 if east < west else east  # crosses the antimeridian
        lons = [(lon + 180) % 360 - 180 if lon > 180 else lon for lon in (west, (west + east) / 2, east)]
    else:
        west, south = min(b[0] for b in boxes), min(b[1] for b in boxes)
        east, north = max(b[2] for b in boxes), max(b[3] for b in boxes)
        lons = [west, (west + east) / 2, east]
    return [(lon, lat) for lon in lons for lat in (south, (south + north) / 2, north)]


def sweep(osr, proj, areas, progress):
    """GDAL samples, references and candidate definitions for every code."""
    ecef = spatial_reference(osr, 4978)
    cases = {}
    started = time.monotonic()
    for count, (code, boxes) in enumerate(areas.items()):
        try:
            cases[code] = sweep_code(osr, proj, ecef, code, boxes)
        except RuntimeError as error:  # GDAL cannot even instantiate the CRS: no evidence
            cases[code] = {"code": code, "name": "", "points": [], "gdal": [], "operations": [],
                           "candidates": [], "error": str(error)}
        if progress and count % 500 == 0:
            print(f"  {count}/{len(areas)} codes, {time.monotonic() - started:.0f} s", file=sys.stderr, flush=True)
    return cases


def sweep_code(osr, proj, ecef, code, boxes):
    """One code's samples, GDAL references and ranked candidate definitions."""
    source = spatial_reference(osr, code)
    geographic = source.CloneGeogCS()
    units = geographic.GetAngularUnits()
    prime = float(source.GetAttrValue("PRIMEM", 1) or 0) * units * 180 / math.pi
    geographic.SetAngularUnits("degree", math.pi / 180)
    geographic.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    geographic.PromoteTo3D(None)
    seed = strict_transform(osr, geographic, source)
    reference = strict_transform(osr, source, ecef)
    points = [transform_point(seed, (lon - prime, lat, h)) for lon, lat in lattice(boxes) for h in HEIGHTS]
    expected = [transform_point(reference, p) for p in points]
    operations = proj.operations(code)
    base = source.Clone()
    base.DemoteTo2D(None)
    candidates, seen = [], set()
    for index, op in enumerate(operations):
        shift = candidate_shift(op)
        if shift is None:
            continue
        definition = base.Clone()
        try:
            definition.SetTOWGS84(*shift)
        except RuntimeError:
            continue
        # Either export can fail (a method PROJ strings or WKT1 cannot express).
        text, wkt1 = exported(definition.ExportToProj4), exported(definition.ExportToWkt, ["FORMAT=WKT1_GDAL"])
        if (text, wkt1) in seen:
            continue
        seen.add((text, wkt1))
        for form, value in (("proj", text), ("wkt1", wkt1)):
            if value:
                candidates.append({"form": form, "definition": value, "operation": index,
                                   "null": not any(shift)})
    return {"code": code, "name": source.GetName(), "points": points, "gdal": expected,
            "operations": operations, "candidates": candidates}


def run_engine(cases, scratch, cargo):
    """Evaluate every candidate with the crate's resolver (an ignored lib test)."""
    inputs = [{"code": c["code"], "points": c["points"],
               "definitions": [{"form": "epsg", "definition": f"EPSG:{c['code']}"}] + c["candidates"]}
              for c in cases.values() if complete(c)]
    path_in, path_out = scratch / "engine-cases.json", scratch / "engine-results.json"
    path_in.write_text(json.dumps(inputs))
    env = dict(CALLER_ENV, RUSTY_TILES_CRS_ALLOWLIST_CASES=str(path_in),
               RUSTY_TILES_CRS_ALLOWLIST_RESULTS=str(path_out))
    command = shlex.split(cargo) + ["test", "--release", "--locked", "--lib",
                                    "crs::allowlist::generator::engine_results", "--", "--ignored", "--exact"]
    print("+", " ".join(command), file=sys.stderr, flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True, stdout=sys.stderr)
    return {r["code"]: r["results"] for r in json.loads(path_out.read_text())}


def complete(case):
    return (len(case["points"]) == SAMPLES and all(p is not None for p in case["points"])
            and all(e is not None for e in case["gdal"]))


def max_error(answers, expected):
    """Largest ECEF error, or None unless every sample answered (refusal never passes)."""
    if len(answers) != SAMPLES or any(a is None for a in answers):
        return None
    return max(math.dist(a, e) for a, e in zip(answers, expected))


def dynamic_codes(osr, codes):
    """Codes whose datum is itself a dynamic reference frame (ITRF, WGS 84 (Gxxxx), WGS 72...).

    Without a coordinate epoch their operation to WGS 84 is a convention, so they stay
    refused even where GDAL answers; native PROJ handles them with an epoch. Datum
    ensembles (WGS 84, ETRS89) are not dynamic frames: EPSG states their accuracy, and
    GDAL's `IsDynamic()` (true for an ensemble of dynamic members) is not used."""
    dynamic = set()
    for code in codes:
        try:
            wkt = spatial_reference(osr, code).ExportToWkt(["FORMAT=WKT2_2019"])
        except RuntimeError:
            continue
        if "DYNAMIC[" in wkt:
            dynamic.add(code)
    return dynamic


def select(cases, results, dynamic=frozenset()):
    """First passing definition per code: built-in EPSG, then ranked candidates.

    A dynamic-datum code is refused even when it passes; it is reported as such."""
    builtin, table, failed = select_passing(cases, results)
    for row in [r for r in table if r["code"] in dynamic]:
        table.remove(row)
        failed[row["code"]] = "dynamic datum (passed the samples, refused without a coordinate epoch)"
    for code in dynamic - {r["code"] for r in table} - set(failed):
        failed[code] = "dynamic datum"
    return builtin, table, failed


def select_passing(cases, results):
    builtin, table, failed = [], [], {}
    for code, case in cases.items():
        if not complete(case):
            failed[code] = "incomplete GDAL samples or references"
            continue
        outcomes = results[code]
        errors = [max_error(o["ok"], case["gdal"]) for o in outcomes]
        if errors[0] is not None and errors[0] <= TOLERANCE_M:
            builtin.append(code)
            continue
        for candidate, error in zip(case["candidates"], errors[1:]):
            if error is not None and error <= TOLERANCE_M:
                op = case["operations"][candidate["operation"]]
                kind = "conversion" if not op["datum_operation"] else "null" if candidate["null"] else "helmert"
                table.append({"code": code, "accuracy": op["accuracy"], "kind": kind,
                              "operation": op["datum_operation"], "operation_id": op["datum_operation_id"],
                              "definition": candidate["definition"], "max_error_m": error})
                break
        else:
            answered = [e for e in errors[1:] if e is not None]
            failed[code] = (f"best candidate {min(answered):.6f} m" if answered else
                            "no candidate definition" if not case["candidates"] else "resolver refused every candidate")
    return builtin, table, failed


def clean(text):
    if any(c in text for c in "\t\n\r"):
        raise ValueError(f"table field contains a tab or line break: {text!r}")
    return text


def render(table, builtin, versions):
    epsg, proj_version, gdal_version = versions
    lines = [
        f"# {FORMAT}",
        f"# epsg={epsg} proj={proj_version} gdal={gdal_version}",
        f"# criterion: all {SAMPLES} samples (3x3 area-of-use lattice, heights 0 m and 1000 m) within "
        f"{TOLERANCE_M} m ECEF of GDAL only-best, no-ballpark, network-off, grid-free PROJ",
        f"# codes={len(table)} builtin={len(builtin)}",
        "# code\taccuracy_m\tkind\toperation_id\toperation\tdefinition",
    ]
    for row in table:
        accuracy = "unknown" if row["accuracy"] < 0 else repr(float(row["accuracy"]))
        lines.append("\t".join([str(row["code"]), accuracy, row["kind"], clean(row["operation_id"]),
                                clean(row["operation"]), clean(row["definition"])]))
    return "\n".join(lines) + "\n"


def fixtures(cases, versions):
    """GDAL ECEF references at three lattice samples of each fixture code."""
    rows = []
    for code in FIXTURE_CODES:
        case = cases[code]
        # South-west corner at 0 m, centre at 0 m and north-east corner at 1000 m.
        for index in (0, 8, 17):
            rows.append({"code": code, "source": case["points"][index], "ecef": case["gdal"][index]})
    epsg, proj_version, gdal_version = versions
    return {"note": "GDAL only-best, no-ballpark, network-off EPSG:<code> to EPSG:4978; written by "
                    "scripts/crs-allowlist/generate.py", "epsg": epsg, "proj": proj_version,
            "gdal": gdal_version, "samples": rows}


def generate(args):
    with tempfile.TemporaryDirectory(prefix="crs-allowlist-") as scratch:
        scratch = args.work.resolve() if args.work else pathlib.Path(scratch)
        (scratch / "home").mkdir(parents=True, exist_ok=True)
        data = isolate_environment(scratch / "home")
        gdal, osr = gdal_modules()
        library = args.libproj or next((str(p) for p in [pathlib.Path(os.environ.get("CONDA_PREFIX", "/nonexistent")) / "lib/libproj.so"] if p.exists()), None) or ctypes.util.find_library("proj")
        if not library:
            sys.exit("error: cannot find libproj; pass --libproj")
        proj = Proj(library)
        proj_version = f"{osr.GetPROJVersionMajor()}.{osr.GetPROJVersionMinor()}.{osr.GetPROJVersionMicro()}"
        epsg, areas = inventory(osr, data / "proj.db")
        versions = (epsg, proj_version, gdal.__version__)
        print(f"GDAL {gdal.__version__}, PROJ {proj_version} ({library}), EPSG {epsg}: {len(areas)} active codes",
              file=sys.stderr, flush=True)
        started = time.monotonic()
        saved = scratch / "sweep.json"
        if args.reuse_sweep and saved.is_file():
            stored = json.loads(saved.read_text())
            if stored["versions"] != list(versions):
                sys.exit(f"error: {saved} was swept with {stored['versions']}")
            cases = {int(code): case for code, case in stored["cases"].items()}
        else:
            cases = sweep(osr, proj, areas, progress=True)
            if args.work:
                saved.write_text(json.dumps({"versions": list(versions), "cases": cases}))
        swept = time.monotonic() - started
        results = run_engine(cases, scratch, args.cargo)
        dynamic = dynamic_codes(osr, cases)
        builtin, table, failed = select(cases, results, dynamic)
        summary = {"versions": dict(zip(("epsg", "proj", "gdal"), versions)), "active": len(areas),
                   "accepted": len(table) + len(builtin), "table": len(table), "builtin": len(builtin),
                   "kinds": {k: sum(r["kind"] == k for r in table) for k in ("conversion", "null", "helmert")},
                   "unknown_accuracy": sum(r["accuracy"] < 0 for r in table),
                   "dynamic_codes": len(dynamic),
                   "dynamic_refused_after_passing": sum(v.startswith("dynamic datum (passed") for v in failed.values()),
                   "max_error_m": max((r["max_error_m"] for r in table), default=0.0),
                   "sweep_seconds": round(swept, 1), "accepted_codes": sorted([r["code"] for r in table] + builtin),
                   "refused": {str(k): v for k, v in sorted(failed.items())}}
        return render(table, builtin, versions), fixtures(cases, versions), summary


def read_table(path):
    return gzip.decompress(path.read_bytes()).decode()


def header_versions(text):
    fields = dict(item.split("=", 1) for item in text.splitlines()[1].lstrip("# ").split())
    return fields["epsg"], fields["proj"], fields["gdal"]


def compare_fixtures(old, new):
    if {k: old[k] for k in ("epsg", "proj", "gdal")} != {k: new[k] for k in ("epsg", "proj", "gdal")}:
        return ["fixture versions differ"]
    problems = []
    for a, b in zip(old["samples"], new["samples"]):
        if a["code"] != b["code"] or math.dist(a["source"], b["source"]) > 1e-9 or math.dist(a["ecef"], b["ecef"]) > 1e-6:
            problems.append(f"fixture sample for EPSG:{a['code']} drifted")
    if len(old["samples"]) != len(new["samples"]):
        problems.append("fixture sample count differs")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", choices=("write", "check", "versions"))
    parser.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"),
                        help="cargo command, possibly with a wrapper (default: $CARGO or cargo)")
    parser.add_argument("--libproj", help="libproj shared library GDAL uses (default: $CONDA_PREFIX/lib/libproj.so)")
    parser.add_argument("--summary", type=pathlib.Path, help="also write a JSON summary (counts, refusals) here")
    parser.add_argument("--work", type=pathlib.Path, help="keep intermediate files (GDAL sweep, engine results) here")
    parser.add_argument("--reuse-sweep", action="store_true",
                        help="reuse the GDAL sweep saved in --work (same versions) instead of sampling again")
    args = parser.parse_args()
    if args.action == "versions":
        epsg, proj_version, gdal_version = header_versions(read_table(TABLE))
        print(f"epsg={epsg}\nproj={proj_version}\ngdal={gdal_version}")
        return 0
    if args.action == "check":
        committed = read_table(TABLE)
        wanted = header_versions(committed)
    text, reference, summary = generate(args)
    if args.summary:
        args.summary.write_text(json.dumps(summary, indent=1) + "\n")
    print(f"{summary['accepted']} of {summary['active']} codes accepted ({summary['table']} table + "
          f"{summary['builtin']} built-in); worst sampled error {summary['max_error_m'] * 1000:.4f} mm",
          file=sys.stderr)
    if summary["max_error_m"] > TOLERANCE_M:  # cannot happen by construction; keep the gate explicit
        print("error: a table entry exceeds 1 mm", file=sys.stderr)
        return 1
    if args.action == "write":
        TABLE.parent.mkdir(parents=True, exist_ok=True)
        TABLE.write_bytes(gzip.compress(text.encode(), compresslevel=9, mtime=0))
        FIXTURE.write_text(json.dumps(reference, indent=1) + "\n")
        print(f"wrote {TABLE.relative_to(ROOT)} ({TABLE.stat().st_size} bytes) and {FIXTURE.relative_to(ROOT)}",
              file=sys.stderr)
        return 0
    found = header_versions(text)
    if found != wanted:
        print(f"error: table was generated with epsg/proj/gdal {wanted}, this environment has {found}",
              file=sys.stderr)
        return 1
    problems = compare_fixtures(json.loads(FIXTURE.read_text()), reference)
    if text != committed:
        diff = list(difflib.unified_diff(committed.splitlines(), text.splitlines(), "committed", "regenerated",
                                         lineterm="", n=0))
        problems.append(f"table drifted ({sum(l[:1] in '+-' for l in diff[2:])} changed lines):\n" +
                        "\n".join(diff[:40]))
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    if not problems:
        print("table and fixture match a fresh generation", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
