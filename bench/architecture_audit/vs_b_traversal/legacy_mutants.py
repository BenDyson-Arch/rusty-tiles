"""Re-run of the existing validator mutant sets on the integrated VS-B source.

Usage (repository root, clean `src/`):
    python3 -I bench/architecture_audit/vs_b_traversal/legacy_mutants.py OUT_DIR [name ...]

Sets: the C2 production patches (slice 1, H11, slice 3), the I1 production patches, the EM and
VS-A string mutants, and the H23 payload-replay mutants. A mutant whose anchor VS-B removed is
replaced by a port here (`PORTS`): the same defect on the new code, named `port:<original>`.
Each mutant is applied, the release validator and parity tests run (with the slice-3
evidence and forest directories when the environment names them), and `src/` is restored.
Caught: a test fails or the run aborts after compiling; a mutant that does not compile is
`invalid`. Writes OUT_DIR/report.json. Environment as `mutants.py` (VSB_*), plus
C2S3_EVIDENCE_DIR and C2S3_FOREST_DIR.
"""
import glob
import importlib.util
import json
import os
import re
import subprocess
import sys

ROOT = "bench/architecture_audit"
V = "src/validate.rs"
PAYLOAD = "src/content_integrity/payload.rs"
STALE = {
    "c2:skip-ancestors", "c2:skip-content-volume", "c2:skip-external-root-transform",
    "c2:reuse-URI-cache-for-verdicts", "c2:test-undrawn-vertices",
    "c2:slice3/depth-is-chain-length-minus-1", "c2:slice3/derived-detected-in-build-levels-only",
    "c2:slice3/merge-I0-into-R", "c2:slice3/slice1-reuse-URI-cache-for-verdicts",
    "c2:slice3/slice1-skip-external-root-transform", "c2:slice3/units-per-URI",
    "i1:drop-repeated-subtree-check", "em:replay-without-references",
    "em:nested-placement-unrecorded", "c2:h11/trust-declared-bounds",
    # These two apply textually but name the removed tile arena (they do not compile).
    "c2:reverse-tile-order", "c2:slice3/R-transform-before-V_R",
    "c2:slice3/external-content-at-R-level", "c2:slice3/slice1-reverse-tile-order",
    # Develop's H7 moved this check into a shared suffix helper.
    "i1:case-sensitive-suffix",
}
PORTS = [
    # H11: the corner test trusts the declared accessor bounds of viewless accessors.
    ("port:c2:h11/trust-declared-bounds", [
        ("src/content_integrity/payload.rs",
         "            parsed.min = min;\n            parsed.max = max;\n        }\n",
         "            parsed.min = min;\n            parsed.max = max;\n        }\n"
         "        if view.is_none() {\n"
         "            for (bound, out) in [(a.min, &mut parsed.min), (a.max, &mut parsed.max)] {\n"
         "                if let Some(bound) = bound {\n"
         "                    let declared = array::<&RawValue>(Some(bound), slots)?;\n"
         "                    for (raw, o) in declared.iter().zip(out.iter_mut()) {\n"
         "                        *o = serde_json::from_str::<f64>(raw.get()).unwrap_or(*o);\n"
         "                    }\n                }\n            }\n        }\n"),
        ("src/content_integrity/payload/scene.rs",
         "    pub(crate) fn positions(&self) -> &[usize] {\n        self.positions\n    }\n",
         "    pub(crate) fn positions(&self) -> &[usize] {\n        self.positions\n    }\n"
         "    pub(crate) fn extent(&self, accessor: usize) -> ([f64; 3], [f64; 3]) {\n"
         "        let a = &self.draws.accessors[accessor];\n"
         "        (std::array::from_fn(|c| a.min.get(c).copied().unwrap_or(f64::NAN)),\n"
         "         std::array::from_fn(|c| a.max.get(c).copied().unwrap_or(f64::NAN)))\n    }\n"),
        (V, "containment::walk_marked(self.arena, n, self.levels, bitmap, self.credit, |v| {\n"
            "                    raw_vertex(instance.vertex(accessor, v))\n                });",
         "containment::walk_marked_declared(self.arena, n, self.levels, bitmap, self.credit, |v| {\n"
         "                    raw_vertex(instance.vertex(accessor, v))\n                }, {\n"
         "                    let (lo, hi) = instance.extent(accessor);\n"
         "                    let word = |p: [f64; 3]| match instance.format(accessor) {\n"
         "                        PositionFormat::F32 => RawVertex::F32(p.map(|x| x as f32)),\n"
         "                        _ => RawVertex::Int(p.map(|x| x as i32)),\n                    };\n"
         "                    Some((word(lo), word(hi)))\n                });"),
        ("src/validate/containment.rs",
         "    vertex: impl Fn(usize) -> RawVertex,\n) -> Option<(usize, Verdict)> {\n    let diagonal = diagonal_suffices(arena, levels);",
         "    vertex: impl Fn(usize) -> RawVertex,\n) -> Option<(usize, Verdict)> {\n"
         "    walk_marked_declared(arena, n, levels, bitmap, credit, vertex, None)\n}\n"
         "#[allow(clippy::too_many_arguments)]\n"
         "pub(crate) fn walk_marked_declared(\n    arena: &mut Arena,\n    n: Normalization,\n"
         "    levels: &[LevelFacts],\n    bitmap: &[u64],\n    credit: &mut Credit,\n"
         "    vertex: impl Fn(usize) -> RawVertex,\n    declared: Option<(RawVertex, RawVertex)>,\n"
         ") -> Option<(usize, Verdict)> {\n    let diagonal = diagonal_suffices(arena, levels);"),
        ("src/validate/containment.rs", "            .then(|| raw_box(words, first, &vertex))",
         "            .then(|| declared.or_else(|| raw_box(words, first, &vertex)))"),
    ]),
    ("port:c2:reverse-tile-order", [(V, "            try_push(&mut self.levels, fact)?;\n            current = parent;\n        }\n        Ok(())",
        "            try_push(&mut self.levels, fact)?;\n            current = parent;\n        }\n"
        "        let mut path = Vec::new();\n        let mut at = Some(tile);\n"
        "        while let Some(t) = at {\n            path.push(self.path[t].transform);\n"
        "            at = self.path[t].parent;\n        }\n        path.reverse();\n"
        "        for (l, level) in self.levels.iter_mut().enumerate().skip(1) {\n"
        "            level.to_parent = path[l - 1];\n        }\n        Ok(())")]),
    ("port:c2:slice3/R-transform-before-V_R", [(V, "            let index = self.c2.tile(TileFact {\n                transform: None,",
        "            let moved = if level == 0 { self.c2.path[r.tile].transform.take() } else { None };\n"
        "            let index = self.c2.tile(TileFact {\n                transform: moved,")]),
    ("port:c2:slice3/external-content-at-R-level", [(V,
        "            let index = self.c2.tile(TileFact {\n                transform: None,",
        "            let moved = if level == 0 { self.c2.path[r.tile].external.take() } else { None };\n"
        "            let index = self.c2.tile(TileFact {\n                transform: None,"),
        (V, "                    EffectiveVolume::Derived(_) => None,\n                },\n                external: None,",
         "                    EffectiveVolume::Derived(_) => None,\n                },\n                external: moved,")]),
    ("port:i1:case-sensitive-suffix", [("src/content_integrity/implicit.rs",
        "        ends_with_suffix(self.static_tail(), \"json\")",
        "        { let tail = self.static_tail(); tail.len() >= 5 && &tail[tail.len() - 5..] == b\".json\" }")]),
    ("port:c2:skip-ancestors", [(V, "                    Some((Role::Ancestor(level), volume)),",
                                 "                    None,")]),
    ("port:c2:skip-content-volume", [(V, "                volumes: [Some((Role::Own, own)), content, None],",
                                      "                volumes: [Some((Role::Own, own)), None, None],")]),
    ("port:c2:skip-external-root-transform", [(V, "                to_parent: child.transform,",
                                               "                to_parent: child.transform.filter(|_| child.external.is_none()),")]),
    # One stream per URI: later references to an already streamed URI are not streamed.
    ("port:c2:reuse-URI-cache-for-verdicts", [(V, "        self.c2.stream.started = true;\n",
                                               "        self.c2.stream.started = true;\n"
                                               "        if self.repeat_seen(rank) {\n            return Ok(());\n        }\n")]),
    ("port:c2:test-undrawn-vertices", [(V, "            instance.mark(accessor, |v| bitmap[v / 64] |= 1 << (v % 64));",
                                        "            (0..instance.count(accessor)).for_each(|v| bitmap[v / 64] |= 1 << (v % 64));")]),
    ("port:c2:slice3/depth-is-chain-length-minus-1", [(V, "                parent: Some(parent_tile),\n                depth,\n",
                                                       "                parent: Some(parent_tile),\n                depth: depth + 1,\n")]),
    ("port:c2:slice3/derived-detected-in-build-levels-only", [
        (V, "            .map_or(Reason::DerivedImplicitVolume.bit(), shape_bit)", "            .map_or(0, shape_bit)"),
        (V, "        self.build_levels(tile, content)?;\n        self.precheck = self.precheck_step(facts);",
         "        if self.build_levels(tile, content).is_err() {\n"
         "            self.exclude(Reason::DerivedImplicitVolume);\n            return Ok(false);\n        }\n"
         "        self.precheck = self.precheck_step(facts);")]),
    ("port:c2:slice3/merge-I0-into-R", [
        (V, "                transform: None,\n                volume: match &tile.volume {",
         "                transform: if level == 0 { self.c2.path[r.tile].transform } else { None },\n"
         "                volume: match &tile.volume {"),
        (V, "                external: None,\n                parent: Some(parent_tile),",
         "                external: if level == 0 { self.c2.path[r.tile].external } else { None },\n"
         "                parent: if level == 0 { self.c2.path[r.tile].parent } else { Some(parent_tile) },")]),
    # Units per URI, not per reference: a repeat reference is neither registered nor charged.
    ("port:c2:slice3/units-per-URI", [(V, "        if !self.c2.reference(tile, content, &stored.facts)? {",
                                       "        if self.repeat_seen(rank) || !self.c2.reference(tile, content, &stored.facts)? {")]),
    ("port:i1:drop-repeated-subtree-check", [(V, "            if self.subtrees.get(member.rank) {",
                                              "            if false && self.subtrees.get(member.rank) {")]),
    ("port:em:replay-without-references", [(V, "                    self.c2_ref(frame.locals[*tile], *content, *rank, &stored)?;",
                                            "                    let _ = (tile, content, rank, stored);")]),
    ("port:em:nested-placement-unrecorded", [(V, "                (Some(summary), Some(parent)) => self.record(Event::Place(Box::new(Placement {",
                                              "                (Some(summary), Some(parent)) if false => self.record(Event::Place(Box::new(Placement {")]),
]
# `units-per-URI` needs a helper on Check: whether this rank was referenced before.
HELPER = (V, "    /// One C2 reference: registration and precheck step and, while in domain, its inline",
          "    fn repeat_seen(&mut self, rank: u64) -> bool {\n"
          "        thread_local!(static SEEN: std::cell::RefCell<std::collections::HashSet<u64>> = Default::default());\n"
          "        SEEN.with(|s| !s.borrow_mut().insert(rank))\n    }\n"
          "    /// One C2 reference: registration and precheck step and, while in domain, its inline")
H23 = [
    ("h23:omit-member-check", [(PAYLOAD, "    if replay.is_some_and(|admission| member_digest(kind, bytes) != admission.member) {\n"
                                "        return Err(invalid(\"source archive changed during inspection\").into());\n    }\n", "")]),
    ("h23:omit-resource-check", [(PAYLOAD, "    if replay.is_some_and(|admission| resources != admission.resources) {\n"
                                  "        return Err(invalid(\"source archive changed during inspection\").into());\n    }\n", "")]),
    ("h23:omit-resource-length", [(PAYLOAD, "    hash.update((bytes.len() as u64).to_le_bytes());\n", "")]),
    ("h23:hash-truncated-buffer", [(PAYLOAD, "            resource_digest(&mut resolved_hash, uri, &data);",
                                    "            resource_digest(&mut resolved_hash, uri, &data[..n]);")]),
    ("h23:repeat-value-scan", [(PAYLOAD, "        if replay.is_none() {\n            let mut min = allocation(parsed.components, f64::INFINITY)?;",
                                "        if true {\n            let mut min = allocation(parsed.components, f64::INFINITY)?;")]),
    ("h23:repeat-ktx2-inspection", [(PAYLOAD, "None if replay.is_none() => {", "None if true => {"),
                                     (PAYLOAD, "if flags != 0 && replay.is_none() {", "if flags != 0 {")]),
    ("h23:verify-resources-after-consumer", [
        (PAYLOAD, "    if replay.is_some_and(|admission| resources != admission.resources) {\n"
                  "        return Err(invalid(\"source archive changed during inspection\").into());\n    }\n", ""),
        (PAYLOAD, "        .map_err(SceneError::Consumer)?;\n",
                  "        .map_err(SceneError::Consumer)?;\n"
                  "    if replay.is_some_and(|admission| resources != admission.resources) {\n"
                  "        return Err(invalid(\"source archive changed during inspection\").into());\n    }\n")]),
    ("h23:omit-first-value-scan", [(PAYLOAD, "        if replay.is_none() {\n            let mut min = allocation(parsed.components, f64::INFINITY)?;",
                                    "        if false {\n            let mut min = allocation(parsed.components, f64::INFINITY)?;")]),
]
TESTS = ["validate::", "content_integrity::", "c2_oracle_parity", "i1_oracle_parity", "archive3tz",
         "payload_context_acceptance"]


def sh(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def load(path):
    spec = importlib.util.spec_from_file_location("m" + str(abs(hash(path))), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def string_sets():
    out = []
    for label, path, default in [("vsa", f"{ROOT}/vs_a_catalog/mutants.py", None),
                                 ("em", f"{ROOT}/external_memo/mutants.py", V)]:
        for name, edits in load(path).MUTANTS:
            if name == "clean":
                continue
            edits = [e if len(e) == 3 else (default,) + tuple(e) for e in edits]
            out.append((f"{label}:{name}", ("edits", edits)))
    return out


def patch_sets():
    out = []
    for label, directory in [("c2", f"{ROOT}/c2_decoded_containment/acceptance/production-mutants"),
                             ("i1", f"{ROOT}/implicit_read/acceptance/production-mutants")]:
        for path in sorted(glob.glob(f"{directory}/**/*.patch", recursive=True)):
            name = os.path.relpath(path, directory)[:-len(".patch")]
            out.append((f"{label}:{name}", ("patch", path)))
    return out


def main(argv):
    out = os.path.abspath(argv[0])
    names = set(argv[1:])
    os.makedirs(out, exist_ok=True)
    if sh(["git", "diff", "--quiet", "--", "src"]).returncode:
        sys.exit("src/ must be clean")
    home = os.path.expanduser("~")
    target = os.environ.get("CARGO_TARGET_DIR", f"{home}/.cache/rt-target-vsb")
    lock = os.environ.get("VSB_LOCK", f"{home}/.cache/rusty-tiles-cargo-d.lock")
    cpus = os.environ.get("VSB_CPUS", "4-7")
    timeout = int(os.environ.get("VSB_TIMEOUT", "2400"))
    jobs = os.environ.get("VSB_JOBS", "2")
    mutants = [("clean", ("edits", []))] + patch_sets() + string_sets() + \
        [(n, ("edits", e)) for n, e in PORTS + H23]
    report = []
    for name, (kind, spec) in mutants:
        if names and name not in names:
            continue
        if name in STALE:
            report.append({"name": name, "replacedBy": "port:" + name})
            continue
        originals = {}
        applied = True
        if kind == "patch":
            applied = sh(["git", "apply", spec]).returncode == 0
        else:
            helper = name in ("port:c2:slice3/units-per-URI", "port:c2:reuse-URI-cache-for-verdicts")
            edits = list(spec) + ([HELPER] if helper else [])
            for path, old, new in edits:
                source = originals.setdefault(path, open(path).read())
                current = open(path).read()
                if current.count(old) != 1:
                    applied = False
                    break
                open(path, "w").write(current.replace(old, new))
        try:
            if not applied:
                report.append({"name": name, "caught": False, "invalid": True, "reason": "does not apply"})
                print(name, "DOES NOT APPLY", flush=True)
                continue
            patch = sh(["git", "diff", "--", "src"]).stdout
            cargo = ["nice", "-n", "10", "taskset", "-c", cpus, "cargo", "test", "--locked", "--release",
                     "--lib", "-j", jobs, "--target-dir", target, "--", "--test-threads=2"] + TESTS
            try:
                run = sh(["flock", "-o", lock, "sh", "-c", 'touch src/lib.rs && exec "$@"', "sh"] + cargo,
                         timeout=timeout)
                stdout, code = run.stdout + run.stderr, run.returncode
            except subprocess.TimeoutExpired as expired:
                stdout, code = f"TIMEOUT after {timeout} s\n{expired.stdout or ''}", -1
            with open(os.path.join(out, name.replace("/", "_").replace(":", "_") + ".log"), "w") as log:
                log.write(stdout)
            failing = re.findall(r"^test (\S+) \.\.\. FAILED$", stdout, re.M)
            invalid = "could not compile" in stdout
            aborted = bool(code) and "test result:" not in stdout and not invalid
            caught = (bool(failing) or aborted) and not invalid
            report.append({"name": name, "caught": caught, "invalid": invalid, "failing": failing,
                           "aborted": aborted, "patch": patch})
            print(name, "INVALID" if invalid else "caught" if caught else "NOT caught", failing[:4], flush=True)
        finally:
            if kind == "patch" and applied:
                sh(["git", "checkout", "--", "src"])
            for path, source in originals.items():
                open(path, "w").write(source)
        with open(os.path.join(out, "report.json"), "w") as f:
            json.dump({"mutants": report}, f, indent=1)
    run = [r for r in report if "replacedBy" not in r]
    ok = all(r.get("caught") == (r["name"] != "clean") and not r.get("invalid") for r in run)
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump({"ok": ok, "mutants": report}, f, indent=1)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
