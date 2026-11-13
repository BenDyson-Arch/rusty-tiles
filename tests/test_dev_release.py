"""scripts/dev_release.sh against a fake GitHub API: no 404 window, and a proven tag move.

Fake `gh` and `curl` executables keep the releases, assets and tag refs in a JSON file.
After every API call the fake records whether each asset the previous build's (or the new
build's) SHA256SUMS names could be downloaded from `releases/download/dev/`, which is the
installer's route.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "dev_release.sh"
OLD_SHA, NEW_SHA = "a" * 40, "b" * 40

FAKE_GH = textwrap.dedent(r'''
    import json, os, re, subprocess, sys
    state_path = os.environ["FAKE_STATE"]
    state = json.load(open(state_path))
    args = sys.argv[1:]
    assert args[0] == "api", args
    args = args[1:]
    method, fields, jq, path = "GET", {}, None, None
    while args:
        a = args.pop(0)
        if a == "-X": method = args.pop(0)
        elif a in ("-f", "-F"):
            key, _, value = args.pop(0).partition("=")
            if value.startswith("@"): value = open(value[1:]).read()
            elif value in ("true", "false"): value = value == "true"
            fields[key] = value
        elif a == "--jq": jq = args.pop(0)
        elif a == "--paginate": pass
        else: path = a
    path = re.sub(r"^repos/[^/]+/[^/]+/", "", path)

    def release_view(r):
        return dict(r, assets=[a for a in state["assets"] if a["release"] == r["id"]])

    def published(tag):
        return next((r for r in state["releases"] if r["tag_name"] == tag and not r["draft"]), None)

    def fail(code):
        sys.stderr.write(f"HTTP {code}\n"); sys.exit(1)

    out = None
    if path == "releases" and method == "GET":
        out = [release_view(r) for r in state["releases"]]
    elif path == "releases" and method == "POST":
        state["next"] += 1
        r = dict(id=state["next"], draft=fields["draft"], prerelease=fields["prerelease"],
                 tag_name=fields["tag_name"], target=fields["target_commitish"], name=fields["name"])
        state["releases"].append(r); out = r
    elif m := re.fullmatch(r"releases/tags/(.+)", path):
        r = published(m.group(1))
        if r is None: fail(404)
        out = release_view(r)
    elif m := re.fullmatch(r"releases/(\d+)/assets", path):
        out = [a for a in state["assets"] if a["release"] == int(m.group(1))]
    elif m := re.fullmatch(r"releases/assets/(\d+)", path):
        asset = next(a for a in state["assets"] if a["id"] == int(m.group(1)))
        if method == "DELETE": state["assets"].remove(asset)
        else:
            assert not any(a["name"] == fields["name"] and a["release"] == asset["release"] for a in state["assets"])
            asset["name"] = fields["name"]
        out = asset
    elif m := re.fullmatch(r"releases/(\d+)", path):
        r = next(r for r in state["releases"] if r["id"] == int(m.group(1)))
        if method == "DELETE":
            state["releases"].remove(r)
            state["assets"] = [a for a in state["assets"] if a["release"] != r["id"]]
        elif method == "PATCH":
            if fields.get("draft") is False and r["draft"]:
                tag = fields.get("tag_name", r["tag_name"])
                if tag not in state["refs"]: state["refs"][tag] = fields.get("target_commitish", r["target"])
            r.update({k: v for k, v in fields.items() if k in ("draft", "prerelease", "tag_name", "name")})
        out = release_view(r)
    elif m := re.fullmatch(r"git/refs?/tags/(.+)", path):
        tag = m.group(1)
        if method == "GET":
            if tag not in state["refs"]: fail(404)
            out = {"object": {"type": "commit", "sha": state["refs"][tag]}}
        elif method == "DELETE":
            state["refs"].pop(tag)
            for r in state["releases"]:  # GitHub turns a release whose tag is gone into a draft
                if r["tag_name"] == tag: r["draft"] = True
        elif method == "PATCH":
            if os.environ.get("FAKE_REFUSE_REF"): fail(422)
            state["refs"][tag] = fields["sha"]
    else:
        raise SystemExit(f"unexpected {method} {path}")

    def downloadable():
        r = published("dev")
        names = {a["name"] for a in state["assets"] if r and a["release"] == r["id"]}
        if "SHA256SUMS" not in names: return False
        sums = next(a for a in state["assets"] if a["release"] == r["id"] and a["name"] == "SHA256SUMS")
        return all(n in names for n in sums["content"].split())
    state["log"].append(downloadable())
    json.dump(state, open(state_path, "w"))
    if out is not None:
        text = json.dumps(out)
        if jq: text = subprocess.run(["jq", "-r", jq], input=text, capture_output=True, text=True, check=True).stdout
        sys.stdout.write(text if text.endswith("\n") or not text else text + "\n")
''')

FAKE_CURL = textwrap.dedent(r'''
    import json, os, re, sys
    state = json.load(open(os.environ["FAKE_STATE"]))
    url = sys.argv[-1]
    data = next(a for a in sys.argv if a.startswith("@"))[1:]
    m = re.search(r"/releases/(\d+)/assets\?name=(.+)$", url)
    release, name = int(m.group(1)), m.group(2)
    assert not any(a["name"] == name and a["release"] == release for a in state["assets"]), name
    state["next"] += 1
    state["assets"].append(dict(id=state["next"], release=release, name=name, content=open(data).read()))
    json.dump(state, open(os.environ["FAKE_STATE"], "w"))
''')


class DevReleaseTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-dev-release-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        for name, body in (("gh", FAKE_GH), ("curl", FAKE_CURL)):
            tool = bin_dir / name
            tool.write_text(f"#!{sys.executable}\n{body}")
            tool.chmod(tool.stat().st_mode | stat.S_IEXEC)
        self.env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", GH_TOKEN="t",
                        GH_REPO="o/r", BUILD_SHA=NEW_SHA, GITHUB_RUN_ID="77",
                        FAKE_STATE=str(self.root / "state.json"))
        (self.root / "notes.md").write_text("notes\n")

    def build(self, version):
        dist = self.root / f"dist-{version}"
        dist.mkdir(exist_ok=True)
        names = [f"rusty-tiles-{version}-{t}.tgz" for t in ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin")]
        names += [f"rusty-tiles-{version}-x86_64-pc-windows-msvc.zip", f"rusty_tiles-{version}-cp310-abi3-any.whl"]
        for name in names:
            (dist / name).write_text(f"{name} bytes")
        (dist / "SHA256SUMS").write_text("\n".join(names) + "\n")
        return dist, names

    def state(self, releases=(), assets=(), refs=None):
        state = {"next": 100, "releases": list(releases), "assets": list(assets), "refs": refs or {}, "log": []}
        Path(self.env["FAKE_STATE"]).write_text(json.dumps(state))

    def published_old_build(self):
        _, names = self.build("0.4.1-dev.1")
        assets = [dict(id=i, release=1, name=n, content="x") for i, n in enumerate(names, 10)]
        assets.append(dict(id=50, release=1, name="SHA256SUMS", content="\n".join(names)))
        self.state([dict(id=1, draft=False, prerelease=True, tag_name="dev", target=OLD_SHA,
                         name="Development build")], assets, {"dev": OLD_SHA})

    def run_script(self, dist, **env):
        return subprocess.run(["bash", str(SCRIPT), str(self.root / "notes.md"), str(dist)],
                              env=dict(self.env, **env), capture_output=True, text=True)

    def final(self):
        return json.loads(Path(self.env["FAKE_STATE"]).read_text())

    def test_update_in_place_never_breaks_downloads_and_proves_the_tag(self):
        self.published_old_build()
        dist, names = self.build("0.4.1-dev.2")
        result = self.run_script(dist)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Moved tag dev to " + NEW_SHA, result.stdout)
        self.assertIn(f"git/ref/tags/dev: commit {NEW_SHA}", result.stdout)
        state = self.final()
        # Downloadable after every API call but one: deleting the old SHA256SUMS just before
        # the new one takes its name (GitHub asset names are unique within a release).
        self.assertEqual(state["log"].count(False), 1, state["log"])
        gap = state["log"].index(False)
        self.assertTrue(state["log"][gap + 1])
        self.assertEqual(state["refs"], {"dev": NEW_SHA})
        release = [r for r in state["releases"] if r["tag_name"] == "dev"]
        self.assertEqual(len(release), 1)
        self.assertEqual(release[0]["id"], 1)
        self.assertEqual(sorted(a["name"] for a in state["assets"]), sorted(names + ["SHA256SUMS"]))

    def test_rebuild_of_the_same_version_replaces_each_asset(self):
        self.published_old_build()
        dist, names = self.build("0.4.1-dev.1")
        result = self.run_script(dist)
        self.assertEqual(result.returncode, 0, result.stderr)
        state = self.final()
        self.assertEqual(sorted(a["name"] for a in state["assets"]), sorted(names + ["SHA256SUMS"]))
        self.assertTrue(all("bytes" in a["content"] or a["name"] == "SHA256SUMS" for a in state["assets"]))

    def test_refused_tag_move_falls_back_to_replacing_the_release(self):
        self.published_old_build()
        dist, names = self.build("0.4.1-dev.2")
        result = self.run_script(dist, FAKE_REFUSE_REF="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("refused to move tag dev", result.stdout)
        state = self.final()
        self.assertEqual(state["refs"], {"dev": NEW_SHA})
        self.assertGreater(state["log"].count(False), 1)  # this path has the longer 404 window
        self.assertEqual(sorted(a["name"] for a in state["assets"]), sorted(names + ["SHA256SUMS"]))

    def test_first_build_creates_the_release(self):
        self.state()
        dist, names = self.build("0.4.1-dev.2")
        result = self.run_script(dist)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No published dev release", result.stdout)
        self.assertEqual(self.final()["refs"], {"dev": NEW_SHA})


if __name__ == "__main__":
    unittest.main()
