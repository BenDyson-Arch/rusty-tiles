"""Production mutant driver for the C2 oracle-parity acceptance test.

Usage: python3 -I mutants.py [name ...]   (no names = all)
Applies each mutant as exact string replacements in the worktree, saves the
`git diff` against the source commit, runs the parity test, records the
disagree list and reverts src/.
"""
import hashlib
import json
import os
import subprocess
import sys

WT = "/tmp/rusty-tiles-c2-mutants"
SRC_COMMIT = "1c9783b"
ACC = "bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants"
CACHE = "/home/bend/.cache/c2-mutants"
OUT = CACHE + "/out"

C = "src/validate/containment.rs"
DY = "src/validate/containment/dyadic.rs"
V = "src/validate.rs"
SC = "src/content_integrity/payload/scene.rs"

MUTANTS = [
    {
        "name": "add-1e-6-slack",
        "oracle": True,
        "what": "box membership compares against h + 1e-6 instead of h",
        "edits": [(C,
                   "                d.float(input, half(b, j).abs());\n",
                   "                d.float(input, half(b, j).abs() + 1e-6);\n")],
    },
    {
        "name": "omit-U",
        "oracle": True,
        "what": "the y-up to z-up U step is not applied in the walk",
        "edits": [(C,
                   "    arena.d.neg(p2);\n    let mut p = [p0, p2, p1];\n",
                   "    let mut p = [p0, p1, p2];\n")],
    },
    {
        "name": "double-U",
        "oracle": True,
        "what": "U is applied twice: (x, y, z) -> (x, -y, -z)",
        "edits": [(C,
                   "    arena.d.neg(p2);\n    let mut p = [p0, p2, p1];\n",
                   "    arena.d.neg(p2);\n    let mut p = [p0, p2, p1];\n"
                   "    arena.d.neg(p[2]);\n    p = [p[0], p[2], p[1]];\n")],
    },
    {
        "name": "reverse-tile-order",
        "oracle": True,
        "what": "level l takes the transform of the tile at chain position l-1 counted from the "
                "root (oracle: path[distance - 1]) instead of the child tile on the reference's side",
        "edits": [(V,
                   "            try_push(&mut self.levels, fact)?;\n            current = parent;\n        }\n        Ok(())\n",
                   "            try_push(&mut self.levels, fact)?;\n            current = parent;\n        }\n"
                   "        let mut path = Vec::new();\n"
                   "        let mut at = Some(tile);\n"
                   "        while let Some(t) = at {\n"
                   "            path.push(self.tiles[t].transform);\n"
                   "            at = self.tiles[t].parent;\n"
                   "        }\n"
                   "        path.reverse();\n"
                   "        for (l, level) in self.levels.iter_mut().enumerate().skip(1) {\n"
                   "            level.to_parent = path[l - 1];\n"
                   "        }\n"
                   "        Ok(())\n")],
    },
    {
        "name": "skip-ancestors",
        "oracle": True,
        "what": "ancestor tile volumes are not placed in the level chain",
        "edits": [(V,
                   "                    Some((Role::Ancestor(level), self.tiles[parent].volume)),\n",
                   "                    None,\n")],
    },
    {
        "name": "skip-external-root-transform",
        "oracle": True,
        "what": "an external-tileset root's transform is not a chain step (production has no "
                "external-root flag on TileFact; the root is recognised by its referencing content volume)",
        "edits": [(V,
                   "                to_parent: child.transform,\n",
                   "                to_parent: child.transform.filter(|_| child.external.is_none()),\n")],
    },
    {
        "name": "test-undrawn-vertices",
        "oracle": True,
        "what": "every POSITION vertex is marked instead of the drawn indices",
        "edits": [(V,
                   "            instance.mark(accessor, |v| bitmap[v / 64] |= 1 << (v % 64));\n",
                   "            (0..instance.count(accessor)).for_each(|v| bitmap[v / 64] |= 1 << (v % 64));\n")],
    },
    {
        "name": "raw-quaternion-polynomial",
        "oracle": True,
        "what": "TRS L uses the raw quaternion polynomial H without the |q|^2 normalisation (n = 1)",
        "edits": [(C,
                   "    d.add(n, xx, yy);\n    d.add(n, n, zz);\n    d.add(n, n, ww);\n",
                   "    d.int(n, 1);\n")],
    },
    {
        "name": "default-to-scene-0",
        "oracle": True,
        "what": "without a scene field, scene 0 is selected whenever scenes is non-empty",
        "edits": [(SC,
                   "        None if scenes.len() == 1 => Some(0),\n",
                   "        None if !scenes.is_empty() => Some(0),\n")],
    },
    {
        "name": "reuse-URI-cache-for-verdicts",
        "oracle": True,
        "what": "a URI streamed for one reference is not streamed again for later references",
        "edits": [(V,
                   "        for reference in &references {\n            self.c2.build_levels(reference.tile, reference.content)?;\n",
                   "        let mut streamed = HashSet::new();\n"
                   "        for reference in &references {\n"
                   "            if !streamed.insert(reference.uri) {\n"
                   "                continue;\n"
                   "            }\n"
                   "            self.c2.build_levels(reference.tile, reference.content)?;\n")],
    },
    {
        "name": "rounded-f64-point-as-exact",
        "oracle": True,
        "what": "box test rounds the point P/W to binary64 and compares that rounded point exactly",
        "edits": [
            (DY,
             "    pub(super) fn copy(&mut self, dst: D, src: D) {\n",
             "    /// A binary64 approximation of a value (top two limbs).\n"
             "    pub(super) fn to_f64(&self, d: D) -> f64 {\n"
             "        let h = self.head(d);\n"
             "        if h.len == 0 {\n"
             "            return 0.0;\n"
             "        }\n"
             "        let k = h.len.min(2);\n"
             "        let m = (h.len - k..h.len)\n"
             "            .rev()\n"
             "            .fold(0f64, |m, i| m * 18446744073709551616.0 + self.v[h.at + i] as f64);\n"
             "        let e = h.exp + 64 * (h.len - k) as i64;\n"
             "        let x = m * 2f64.powi((e / 2) as i32) * 2f64.powi((e - e / 2) as i32);\n"
             "        if h.neg {\n"
             "            -x\n"
             "        } else {\n"
             "            x\n"
             "        }\n"
             "    }\n"
             "    pub(super) fn copy(&mut self, dst: D, src: D) {\n"),
            (C,
             "            (VolumeFacts::Box(b), _) => {\n"
             "                d.float(input, half(b, j).abs());\n"
             "                d.mul(hw, input, w);\n",
             "            (VolumeFacts::Box(b), _) => {\n"
             "                let x = d.to_f64(p[j]) / d.to_f64(w);\n"
             "                d.float(dd, x);\n"
             "                d.float(input, c[j]);\n"
             "                d.sub(dd, dd, input);\n"
             "                d.float(hw, half(b, j).abs());\n"),
        ],
    },
    {
        "name": "rounded-normalized-quotient",
        "oracle": True,
        "what": "normalised integer words become the rounded binary64 quotient v/k with W = 1, "
                "instead of the homogeneous exact k",
        "edits": [(C,
                   "            RawVertex::Int(v) if signed && v[c] <= -k => arena.d.int(x[c], i64::from(-k)),\n"
                   "            RawVertex::Int(v) => arena.d.int(x[c], i64::from(v[c])),\n"
                   "        }\n"
                   "    }\n"
                   "    arena.d.int(x[3], i64::from(k));\n",
                   "            RawVertex::Int(v) if signed && v[c] <= -k => arena.d.int(x[c], -1),\n"
                   "            RawVertex::Int(v) => finite &= arena.d.float(x[c], f64::from(v[c]) / f64::from(k)),\n"
                   "        }\n"
                   "    }\n"
                   "    arena.d.int(x[3], 1);\n")],
    },
    # ---- production-specific ----
    {
        "name": "volumes-in-slot-order",
        "oracle": False,
        "paired": "external-content-volume-excludes",
        "what": "a level's volumes are tested in slot order (ancestor slot before externalContent) "
                "instead of Role order",
        "edits": [(C,
                   "        for want in 0..4 {\n"
                   "            for (role, v) in level.volumes.iter().flatten() {\n"
                   "                if rank(*role) == want && !inside(arena, p, w, v) {\n"
                   "                    return Verdict::Outside {\n"
                   "                        role: *role,\n"
                   "                        level: l as u32,\n"
                   "                    };\n"
                   "                }\n"
                   "            }\n"
                   "        }\n",
                   "        for (role, v) in level.volumes.iter().flatten() {\n"
                   "            if !inside(arena, p, w, v) {\n"
                   "                return Verdict::Outside {\n"
                   "                    role: *role,\n"
                   "                    level: l as u32,\n"
                   "                };\n"
                   "            }\n"
                   "        }\n")],
    },
    {
        "name": "drop-signed-clamp",
        "oracle": False,
        "paired": "W08-signed-min-normalization-clamp",
        "what": "signed normalised words at or below -k are not clamped to -1",
        "edits": [(C,
                   "            RawVertex::Int(v) if signed && v[c] <= -k => arena.d.int(x[c], i64::from(-k)),\n",
                   "")],
    },
    {
        "name": "precheck-sum-cap-off-by-one",
        "oracle": False,
        "paired": "work-containmentWorkUnits-at-sum",
        "what": "running work/bytes/decoded totals must be strictly below the cap (< instead of <=)",
        "edits": [(V,
                   "                .filter(|sum| *sum <= cap)\n",
                   "                .filter(|sum| *sum < cap)\n")],
    },
    {
        "name": "precheck-depth-cap-off-by-one",
        "oracle": False,
        "paired": "work-containmentNodeDepth-at-sum",
        "what": "maxNodeDepth >= containmentNodeDepth is a workLimit (>= instead of >)",
        "edits": [(V,
                   "        if facts.max_depth > self.caps.node_depth {\n",
                   "        if facts.max_depth >= self.caps.node_depth {\n")],
    },
    {
        "name": "span-cap-off-by-one",
        "oracle": False,
        "paired": "worked-example-adjusted-containmentSpanBits-29",
        "what": "a span equal to containmentSpanBits is numericSpan (>= instead of >)",
        "edits": [(C,
                   "    s == u32::MAX || u64::from(s) > span_bits\n",
                   "    s == u32::MAX || u64::from(s) >= span_bits\n")],
    },
    {
        "name": "charge-repeated-indices",
        "oracle": False,
        "paired": "work-containmentWorkUnits-at-sum",
        "what": "T is charged per index occurrence (repeats included) instead of per distinct "
                "bitmap vertex (accessor count)",
        "edits": [(V,
                   "        self.t = self.t.saturating_add(instance.vertex_total());\n",
                   "        self.t = self.t.saturating_add(instance.index_total());\n")],
    },
    {
        "name": "skip-content-volume",
        "oracle": False,
        "paired": "contents-array-second-outside",
        "what": "the reference's content boundingVolume is not placed at level 0",
        "edits": [(V,
                   "                volumes: [Some((Role::Own, self.tiles[tile].volume)), content, None],\n",
                   "                volumes: [Some((Role::Own, self.tiles[tile].volume)), None, None],\n")],
    },
]


def sh(cmd, **kw):
    return subprocess.run(cmd, cwd=WT, check=True, capture_output=True, text=True, **kw).stdout


def clean_src():
    sh(["git", "checkout", "--", "src"])
    dirty = [l for l in sh(["git", "status", "--porcelain"]).splitlines()
             if not l[3:].startswith(ACC.rsplit("/", 1)[0] + "/")]
    assert not dirty, dirty


def run_parity(out_json, log):
    env = dict(os.environ)
    cmd = ["flock", "/home/bend/.cache/rusty-tiles-cargo.lock", "env",
           "CARGO_TARGET_DIR=" + CACHE + "/target", "CARGO_BUILD_JOBS=2", "RAYON_NUM_THREADS=2",
           "RUST_TEST_THREADS=2", "TMPDIR=" + CACHE + "/tmp", "C2_PARITY_OUT=" + out_json,
           "taskset", "-c", "0,1", "nice", "-n", "10",
           "cargo", "test", "--locked", "--release", "--lib", "c2_oracle_parity"]
    if os.path.exists(out_json):
        os.remove(out_json)
    with open(log, "w") as f:
        r = subprocess.run(cmd, cwd=WT, stdout=f, stderr=subprocess.STDOUT, env=env)
    return r.returncode


def main(names):
    os.makedirs(os.path.join(WT, ACC), exist_ok=True)
    clean_src()
    for m in MUTANTS:
        if names and m["name"] not in names:
            continue
        name = m["name"]
        for path, old, new in m["edits"]:
            p = os.path.join(WT, path)
            text = open(p).read()
            assert text.count(old) == 1, (name, path, text.count(old))
            open(p, "w").write(text.replace(old, new))
        patch = sh(["git", "diff", SRC_COMMIT, "--", "src"])
        assert patch, name
        patch_path = os.path.join(WT, ACC, name + ".patch")
        open(patch_path, "w").write(patch)
        out_json = os.path.join(OUT, name + ".json")
        rc = run_parity(out_json, os.path.join(OUT, name + ".log"))
        clean_src()
        print(name, "exit", rc, "json", os.path.exists(out_json), flush=True)


if __name__ == "__main__":
    main(sys.argv[1:])
