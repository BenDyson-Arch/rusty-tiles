#!/usr/bin/env python3
"""Instrument an isolated pinned source copy; never edit production files."""
import argparse
import difflib
import hashlib
import io
import json
import pathlib
import subprocess
import tarfile

BASE = "8dfd74bd87dd23c86278e98d736c3f5912c246cf"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--repo", type=pathlib.Path, required=True)
    p.add_argument("--destination", type=pathlib.Path, required=True)
    p.add_argument("--evidence", type=pathlib.Path, required=True)
    args = p.parse_args()
    repo = args.repo.resolve()
    paths = ["src", "Cargo.toml", "Cargo.lock", "build.rs", "bindings/python", "preview", "docs"]
    immutable = paths[:6] + ["docs/schema", "docs/vector-schema"]
    if subprocess.check_output(["git", "diff", BASE, "--", *immutable], cwd=repo):
        raise SystemExit("Production source differs from baseline.")
    if args.destination.exists():
        raise SystemExit("Destination must be fresh to avoid overwriting another probe.")
    args.destination.mkdir(parents=True)
    raw = subprocess.check_output(["git", "archive", BASE, *paths], cwd=repo)
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        archive.extractall(args.destination, filter="data")
    changes = {}

    def change(name, transform):
        path = args.destination / name
        old = path.read_text()
        expected = subprocess.check_output(["git", "show", f"{BASE}:{name}"], cwd=repo)
        assert path.read_bytes() == expected
        new = transform(old)
        assert new != old
        path.write_text(new)
        changes[name] = {
            "baseline_sha256": hashlib.sha256(expected).hexdigest(),
            "instrumented_sha256": hashlib.sha256(new.encode()).hexdigest(),
            "diff": "".join(difflib.unified_diff(old.splitlines(True), new.splitlines(True), f"a/{name}", f"b/{name}", n=0)),
        }

    def publication(old):
        needle = "    if !output.exists() {\n        std::fs::rename(staging, output)?;"
        assert old.count(needle) == 1
        hook = '''    if !output.exists() {
        // AUDIT ONLY: deterministically insert a competing writer in the race window.
        if let Some(record) = std::env::var_os("RT_AUDIT_DIRECTORY_RACE") {
            use std::os::unix::fs::MetadataExt;
            std::fs::create_dir(output)?;
            std::fs::write(record, std::fs::metadata(output)?.ino().to_string())?;
        }
        std::fs::rename(staging, output)?;'''
        return old.replace(needle, hook)

    def accept_io(old):
        needle = '            db.execute_batch("SAVEPOINT feature;").map_err(sql)?;\n            let result = (|| {\n'
        assert old.count(needle) == 1
        hook = '''            db.execute_batch("SAVEPOINT feature;").map_err(sql)?;
            // AUDIT ONLY: an infrastructure error returned by the accept callback.
            let inject_io = std::env::var_os("RT_AUDIT_ACCEPT_IO").is_some()
                && feature.properties.get("audit_storage_failure") == Some(&json!(true));
            let result = (|| {
                if inject_io {
                    return Err(Error::Io(std::io::Error::from_raw_os_error(28)));
                }
'''
        return old.replace(needle, hook)

    change("src/output.rs", publication)
    change("src/vector/pipeline/store.rs", accept_io)
    change("src/lib.rs", lambda old: old + '''
// AUDIT ONLY: expose private publishers to this isolated executable.
pub fn audit_directory(staging: &std::path::Path, output: &std::path::Path) -> Result<(), Error> {
    output::publish_directory(staging, output, false)
}
pub fn audit_archive(staging: &std::path::Path, output: &std::path::Path) -> Result<(), Error> {
    let job = output::Job::begin(output, false)?;
    std::fs::write(output, b"competing archive writer")?;
    job.publish_tree_3tz(staging, None).map(drop)
}
''')
    change("Cargo.toml", lambda old: old + '''
[[example]]
name = "audit-publication"
path = "audit-publication.rs"
''')
    (args.destination / "audit-publication.rs").write_text('''use std::{path::PathBuf, os::unix::fs::MetadataExt};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = &args[1];
    let root = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&root).unwrap();
    let staging = root.join("staging");
    let output = root.join("output");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("tileset.json"), b"{}").unwrap();
    let staging_inode = std::fs::metadata(&staging).unwrap().ino();
    let record = root.join("competitor-inode.txt");
    let result = if mode == "directory" {
        std::env::set_var("RT_AUDIT_DIRECTORY_RACE", &record);
        rusty_tiles::audit_directory(&staging, &output)
    } else {
        rusty_tiles::audit_archive(&staging, &output)
    };
    let category = result.as_ref().err().map(|e| e.category().0);
    let error = result.as_ref().err().map(ToString::to_string);
    let competitor_inode = std::fs::read_to_string(&record).ok();
    let competitor_bytes = if mode == "archive" {std::fs::read_to_string(&output).ok()} else {None};
    println!("{}", serde_json::json!({"mode":mode,"injected_interleaving":true,"force":false,
        "success":result.is_ok(),"error_category":category,"error":error,
        "staging_inode":staging_inode,"competitor_inode":competitor_inode,
        "output_inode":std::fs::metadata(&output).unwrap().ino(),
        "staging_exists_after":staging.exists(),"output_contains_staged_tileset":output.join("tileset.json").exists(),
        "competitor_bytes_after":competitor_bytes}));
}
''')
    args.evidence.mkdir(parents=True, exist_ok=True)
    (args.evidence / "publication-vector-instrumentation.patch").write_text("".join(v["diff"] for v in changes.values()))
    manifest = {"baseline": BASE, "method": "full git archive copy; only copied source instrumented", "files": {k: {a: b for a, b in v.items() if a != "diff"} for k, v in changes.items()}}
    (args.evidence / "publication-vector-instrumentation.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(args.destination)


if __name__ == "__main__":
    main()
