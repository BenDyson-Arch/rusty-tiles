"""VS-C production mutants (the limits surface), author controls.

Usage (repository root, clean `src/`, `bindings/` and `bench/`):
    python3 -I bench/architecture_audit/vs_c_limits/mutants.py OUT_DIR [name ...]

Each mutant is a set of exact single-occurrence string edits. The driver applies one mutant,
runs its suite and restores the files:

- `lib`: the release unit tests of the limit surface (`validate::limits_controls`, the scratch
  mapping control, the VS-B crossover control and the archive caps);
- `cli`: the CLI contract integration test;
- `py`: builds the Python extension and runs the binding's limit-surface test (needs
  `python3` and a writable PY_DIR, default under the target directory).

A mutant is caught when at least one test fails or the run aborts (a timeout counts as
caught). `clean` must fail nothing. Each patch is recorded with its SHA-256 in
OUT_DIR/report.json.

Environment: CARGO_TARGET_DIR (default ~/.cache/rt-target-vsc), VSC_LOCK (default
~/.cache/rusty-tiles-cargo-c.lock), VSC_CPUS (default 16-19), VSC_JOBS (default 4),
VSC_TIMEOUT (default 2400 s). Scratch goes to TMPDIR; set it away from /tmp.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

LIMITS = "src/validate/limits.rs"
TYPES = "src/validate/types.rs"
VALIDATE = "src/validate.rs"
READ = "src/archive3tz/read.rs"
FACADE = "src/archive3tz.rs"
MAIN = "src/main.rs"
PYTHON = "bindings/python/src/lib.rs"
MUTANTS = [
    ("clean", "lib", []),
    ("clean-cli", "cli", []),
    ("clean-py", "py", []),
    # m1: κ_visits 1/8 -> 1/4.
    ("m1-kappa-visits", "lib", [(LIMITS, "(LimitKey::HierarchyVisits, 1 << 18, 1, 8),",
                                 "(LimitKey::HierarchyVisits, 1 << 18, 1, 4),")]),
    # m2: max -> min in resolve.
    ("m2-min-not-max", "lib", [(LIMITS, "None if scaled > floor => (scaled, Derivation::Scaled),",
                                "None if scaled > floor => (floor, Derivation::Scaled),"),
                               (LIMITS, "None => (floor, Derivation::Floor),",
                                "None => (scaled, Derivation::Floor),")]),
    # m3: an override is raised to the scaled value.
    ("m3-override-max", "lib", [(LIMITS, "Some(value) => (value, Derivation::Explicit),",
                                 "Some(value) => (value.max(scaled), Derivation::Explicit),")]),
    # m4: S is the file length.
    ("m4-s-file-length", "lib", [(VALIDATE, "let stored_bytes = archive.stored_bytes();",
                                  "let stored_bytes = before.len();")]),
    # m5: S without the index member.
    ("m5-s-without-index", "lib", [(READ, "let sum = total.checked_add(member);",
                                    "let sum = total.checked_add(if index { 0 } else { member });")]),
    # m6: budgets resolved before admission (S = 0).
    ("m6-resolve-before-admission", "lib", [(VALIDATE, "budgets.resolve(stored_bytes);",
                                             "budgets.resolve(0);")]),
    # m7: a cap defaults to 1 GiB.
    ("m7-cap-default", "lib", [(LIMITS, "*cap = self.overrides[key as usize];",
                                "*cap = self.overrides[key as usize].or(Some(1 << 30));")]),
    # m8: a cap refuses at its value.
    ("m8-cap-at-value", "lib", [(READ, "Some(value) if observed > value => Err(ReadError::Cap {",
                                 "Some(value) if observed >= value => Err(ReadError::Cap {")]),
    # m9: a tie is labelled scaled.
    ("m9-tie-scaled", "lib", [(LIMITS, "None if scaled > floor => (scaled, Derivation::Scaled),",
                               "None if scaled >= floor => (scaled, Derivation::Scaled),")]),
    # m10: an unset cap is serialized as 0.
    ("m10-null-as-zero", "lib", [(TYPES, 'value["limits"] = serde_json::to_value(&self.limits)?;',
                                  'value["limits"] = serde_json::from_str(&serde_json::to_string(&self.limits)?.replace("null", "0"))?;')]),
    # m11: the CLI accepts any key (a Class A key included).
    ("m11-cli-any-key", "cli", [(MAIN, "        .parse::<rusty_tiles::validate::LimitKey>()\n        .map_err(|error| error.to_string())?;",
                                 "        .parse::<rusty_tiles::validate::LimitKey>()\n        .or_else(|_| \"references\".parse::<rusty_tiles::validate::LimitKey>())\n        .map_err(|error| error.to_string())?;")]),
    # m12: Python ignores max_volume.
    ("m12-python-ignores-max-volume", "py", [(PYTHON, "            request = request.max_volume(key, value);",
                                              "            let _ = (key, value);")]),
    # m13: the facade's tmpfs refusal goes back to io.
    ("m13-facade-io", "lib", [(FACADE, "Ok(refusal) => CodecError::Scratch(refusal),",
                               "Ok(refusal) => CodecError::Io(std::io::Error::other(refusal)),")]),
    # m14: no cancellation check per traversal task (C2 references stream inside it).
    ("m14-no-task-check", "lib", [(VALIDATE, "        while let Some(task) = pending.pop() {\n            self.progress.step()?;",
                                   "        while let Some(task) = pending.pop() {")]),
    # m15: an explicit scratch directory is ignored.
    ("m15-scratch-dir-ignored", "lib", [(VALIDATE, "Some(dir) if dir.is_dir() => crate::spill::Placement::In(dir.to_owned()),",
                                         "Some(dir) if dir.is_dir() => crate::spill::Placement::disk(true),")]),
    # m16: liveDocumentBytes is still not serialized.
    ("m16-live-document-skipped", "lib", [(TYPES, "    pub live_document_bytes: u64,",
                                           "    #[serde(skip_serializing)]\n    pub live_document_bytes: u64,")]),
    # The structured `observed` is the counter before the charge.
    ("observed-before-charge", "lib", [(VALIDATE, "None => Err(self.refuse(used.saturating_add(add), message())),",
                                        "None => Err(self.refuse(used, message())),")]),
    # The I1 reservation's amount is dropped from `observed`.
    ("i1-observed-without-reservation", "lib", [(VALIDATE, "u64::try_from(u128::from(used) + amount)",
                                                 "u64::try_from(u128::from(used) + 0 * amount)")]),
    # The accessor planner's archive-budget refusal loses its structure.
    ("planner-unstructured", "lib", [(VALIDATE, "self.remainder_refused = Some(planned);",
                                      "self.remainder_refused = None.filter(|_: &u64| planned > 0);")]),
    # The suffix names the wrong source.
    ("suffix-source", "lib", [(LIMITS, "limit.derivation.as_str()", "Derivation::Floor.as_str()")]),
    # No check after the row sort.
    ("no-rows-check", "lib", [(VALIDATE, "    let payloads = check.rows.finish(names, scratch.clone())?;\n    check.progress.step()?;",
                               "    let payloads = check.rows.finish(names, scratch.clone())?;")]),
]
LIB_TESTS = ["validate::limits_controls", "validate::catalog_controls::scratch_failures",
             "validate::traversal_controls::budgets_scale", "archive3tz::read::tests"]


def sh(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def main(argv):
    out = os.path.abspath(argv[0])
    names = set(argv[1:])
    os.makedirs(out, exist_ok=True)
    if sh(["git", "diff", "--quiet", "--", "src", "bindings"]).returncode:
        sys.exit("src/ and bindings/ must be clean")
    home = os.path.expanduser("~")
    target = os.environ.get("CARGO_TARGET_DIR", f"{home}/.cache/rt-target-vsc")
    lock = os.environ.get("VSC_LOCK", f"{home}/.cache/rusty-tiles-cargo-c.lock")
    cpus = os.environ.get("VSC_CPUS", "16-19")
    timeout = int(os.environ.get("VSC_TIMEOUT", "2400"))
    jobs = os.environ.get("VSC_JOBS", "4")
    py_dir = os.environ.get("PY_DIR", os.path.join(target, "vsc-mutant-py"))
    cargo = ["nice", "-n", "10", "taskset", "-c", cpus, "cargo"]
    suites = {
        "lib": [cargo + ["test", "--locked", "--release", "--lib", "-j", jobs, "--target-dir", target,
                         "--", "--test-threads=2"] + LIB_TESTS],
        "cli": [cargo + ["test", "--locked", "--release", "--test", "cli_contract", "-j", jobs,
                         "--target-dir", target, "--", "--test-threads=2", "validate_"]],
        "py": [cargo + ["build", "--locked", "--release", "-p", "rusty-tiles-python", "-j", jobs,
                        "--target-dir", target],
               ["sh", "-c", f'mkdir -p "{py_dir}" && cp "{target}/release/librusty_tiles.so" '
                            f'"{py_dir}/rusty_tiles.so" && cd bindings/python/tests && '
                            f'PYTHONPATH="{py_dir}" python3 -m unittest -v -k test_validate_limit_surface test_api']],
    }
    report = []
    for name, suite, edits in MUTANTS:
        if names and name not in names:
            continue
        originals = {}
        for path, old, new in edits:
            originals.setdefault(path, open(path).read())
            current = open(path).read()
            assert current.count(old) == 1, (name, old)
            open(path, "w").write(current.replace(old, new))
        try:
            patch = sh(["git", "diff", "--", "src", "bindings"]).stdout
            stdout, code = "", 0
            for command in suites[suite]:
                try:
                    run = sh(["flock", "-o", lock, "sh", "-c", 'touch src/lib.rs && exec "$@"', "sh"] + command,
                             timeout=timeout)
                    stdout += run.stdout + run.stderr
                    code = run.returncode
                except subprocess.TimeoutExpired as expired:
                    stdout += f"TIMEOUT after {timeout} s\n{expired.stdout or ''}"
                    code = -1
                if code:
                    break
            with open(os.path.join(out, f"{name}.log"), "w") as log:
                log.write(stdout)
            failing = re.findall(r"^test (\S+) \.\.\. FAILED$", stdout, re.M)
            failing += re.findall(r"^(test_\S+) \(.*\) \.\.\. (?:FAIL|ERROR)$", stdout, re.M)
            aborted = bool(code) and not failing
        finally:
            for path, source in originals.items():
                open(path, "w").write(source)
        caught = bool(failing) or aborted
        report.append({"name": name, "suite": suite, "caught": caught, "failing": failing,
                       "aborted": aborted, "patchSha256": hashlib.sha256(patch.encode()).hexdigest(),
                       "patch": patch})
        print(name, "caught" if caught else "NOT caught", failing, flush=True)
    ok = all(r["caught"] == (not r["name"].startswith("clean")) for r in report)
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump({"ok": ok, "mutants": report}, f, indent=1)
    shutil.rmtree(py_dir, ignore_errors=True)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
