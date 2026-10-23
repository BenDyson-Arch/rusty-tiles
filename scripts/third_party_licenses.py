#!/usr/bin/env python3
"""Generate and check the Rust third-party licence files that ship with rusty-tiles.

One file per artefact type is committed under licenses/ and bundled into that artefact:

  cli                the default CLI binary (release archives, cargo-binstall)
  native-geospatial  the binary of the container image (also lists its system libraries)
  python             the Python wheel (bindings/python, default features off)

The crate set of each file is the resolved dependency graph of that artefact
(`cargo tree -e no-dev --target all`: normal and build dependencies for every target
platform, so one file serves all release targets). The licence texts are the licence and
NOTICE files each crate publishes in its package. A crate that publishes none gets the SPDX
standard text of its declared licence and a copyright line built from its `authors`.
Identical texts are written once, followed by the crates they cover.

  third_party_licenses.py generate    rewrite licenses/*.md
  third_party_licenses.py check       fail unless licenses/*.md are current

Both fail on any licence expression that the allow-list does not satisfy. `cargo` must be on
PATH; it downloads (never builds) the crates, which takes seconds.
"""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "licenses"

# name -> (cargo tree selectors, file name)
ARTEFACTS = {
    "cli": (["-p", "rusty-tiles"], "default CLI binary"),
    "native-geospatial": (["-p", "rusty-tiles", "--features", "native-geospatial"],
                          "native-geospatial binary (container image)"),
    "python": (["-p", "rusty-tiles-python"], "Python wheel"),
}

# Licences that may ship. Order is the preference when an OR expression offers a choice.
ALLOWED = [
    "MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "Unicode-3.0",
    "Unicode-DFS-2016", "BSL-1.0", "CC0-1.0", "Unlicense", "MPL-2.0",
    "MIT-0",  # already present (borrow-or-share): MIT without the attribution condition
]
ALLOWED_EXCEPTIONS = {"LLVM-exception"}  # Apache-2.0 WITH LLVM-exception
# Crates whose manifest has no SPDX `license` field (only `license-file`).
EXPRESSION_OVERRIDES = {}
# Licence files in a subdirectory of a crate (not found by the top-level scan) that cover code
# the crate bundles. Each must exist, so a crate update that moves one is noticed.
EXTRA_FILES = {"regex-syntax": ["src/unicode_tables/LICENSE-UNICODE"]}
# Bundled code whose licence text is only inside a source header: (SPDX id, copyright line).
EXTRA_SPDX = {"meshopt": [("MIT", "Copyright (c) 2016-2025 Arseny Kapoulkine (vendored meshoptimizer 0.25)")]}
LICENCE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|UNLICEN[CS]E|NOTICE|COPYRIGHT)([-_. ].*)?$", re.I)
LICENCE_DIRS = {"licenses", "licences", "license", "licence", "license-files"}

SPDX = {}  # filled below: SPDX id -> standard text


def spdx(identifier, text):
    SPDX[identifier] = text.strip("\n")


spdx("MIT", """
MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
""")
spdx("BSD-3-Clause", """
BSD 3-Clause License

Redistribution and use in source and binary forms, with or without modification, are permitted
provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of
   conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of
   conditions and the following disclaimer in the documentation and/or other materials
   provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to
   endorse or promote products derived from this software without specific prior written
   permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
""")
spdx("BSD-2-Clause", """
BSD 2-Clause License

Redistribution and use in source and binary forms, with or without modification, are permitted
provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of
   conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of
   conditions and the following disclaimer in the documentation and/or other materials
   provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
""")
spdx("ISC", """
ISC License

Permission to use, copy, modify, and/or distribute this software for any purpose with or
without fee is hereby granted, provided that the above copyright notice and this permission
notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL
THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY
DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF
CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
PERFORMANCE OF THIS SOFTWARE.
""")
spdx("Zlib", """
zlib License

This software is provided 'as-is', without any express or implied warranty. In no event will
the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial
applications, and to alter it and redistribute it freely, subject to the following
restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote
   the original software. If you use this software in a product, an acknowledgment in the
   product documentation would be appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be misrepresented as
   being the original software.
3. This notice may not be removed or altered from any source distribution.
""")
spdx("MIT-0", """
MIT No Attribution

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
""")
spdx("Unlicense", """
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or distribute this software,
either in source code form or as a compiled binary, for any purpose, commercial or
non-commercial, and by any means.

In jurisdictions that recognize copyright laws, the author or authors of this software
dedicate any and all copyright interest in the software to the public domain. We make this
dedication for the benefit of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of relinquishment in perpetuity of
all present and future rights to this software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <https://unlicense.org>
""")

# ---- SPDX expressions ------------------------------------------------------------------

def parse_expression(text):
    """Parse an SPDX expression into ('id', name, exception) / ('and'|'or', [children])."""
    tokens = re.findall(r"\(|\)|[^\s()]+", text.replace("/", " OR "))
    position = 0

    def peek():
        return tokens[position] if position < len(tokens) else None

    def take():
        nonlocal position
        if position >= len(tokens):
            raise ValueError(f"unexpected end of licence expression {text!r}")
        position += 1
        return tokens[position - 1]

    def operand():
        token = take()
        if token == "(":
            node = disjunction()
            if take() != ")":
                raise ValueError(f"unbalanced parentheses in {text!r}")
            return node
        exception = None
        if (peek() or "").upper() == "WITH":
            take()
            exception = take()
        return ("id", token, exception)

    def conjunction():
        nodes = [operand()]
        while (peek() or "").upper() == "AND":
            take()
            nodes.append(operand())
        return nodes[0] if len(nodes) == 1 else ("and", nodes)

    def disjunction():
        nodes = [conjunction()]
        while (peek() or "").upper() == "OR":
            take()
            nodes.append(conjunction())
        return nodes[0] if len(nodes) == 1 else ("or", nodes)

    node = disjunction()
    if position != len(tokens):
        raise ValueError(f"cannot parse licence expression {text!r}")
    return node


def choose(node):
    """The licences to honour: list of (id, exception), or None if the allow-list fails."""
    kind = node[0]
    if kind == "id":
        _, name, exception = node
        if name not in ALLOWED or (exception and exception not in ALLOWED_EXCEPTIONS):
            return None
        return [(name, exception)]
    if kind == "and":
        parts = [choose(child) for child in node[1]]
        return None if None in parts else [item for part in parts for item in part]
    options = [option for option in (choose(child) for child in node[1]) if option]
    if not options:
        return None
    return min(options, key=lambda option: (len(option), [ALLOWED.index(n) for n, _ in option]))


def identifiers(node):
    if node[0] == "id":
        return {node[1]}
    return set().union(*(identifiers(child) for child in node[1]))


# ---- cargo ------------------------------------------------------------------------------

def cargo(*args):
    result = subprocess.run(["cargo", *args], cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        sys.exit(f"error: cargo {' '.join(args)} failed:\n{result.stderr}")
    return result.stdout


def resolved_crates(selectors):
    out = cargo("tree", "--locked", *selectors, "--target", "all", "-e", "no-dev",
                "--prefix", "none", "--format", "{p}")
    crates = set()
    for line in out.splitlines():
        match = re.match(r"^(\S+) v(\S+)(?: \((.*?)\))?", line)
        if not match:
            continue
        name, version, source = match.groups()
        if source and source.startswith("/"):
            continue  # a workspace crate: the project's own code (LICENSE)
        if source and "://" in source:
            sys.exit(f"error: {name} {version} comes from {source}; only crates.io crates are supported")
        crates.add((name, version))
    return crates


def metadata():
    packages = json.loads(cargo("metadata", "--locked", "--all-features", "--format-version", "1"))["packages"]
    return {(p["name"], p["version"]): p for p in packages}


def normalise(data):
    text = data.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")
    text = "\n".join(line.rstrip() for line in text.split("\n"))
    return text.strip("\n").replace("\t", "    ")


def licence_files(package):
    base = pathlib.Path(package["manifest_path"]).parent
    found = {}
    for path in sorted(base.iterdir()):
        if path.is_file() and LICENCE_FILE.match(path.name):
            found[path] = None
        elif path.is_dir() and path.name.lower() in LICENCE_DIRS:
            for inner in sorted(path.iterdir()):
                if inner.is_file():
                    found[inner] = None
    for relative in EXTRA_FILES.get(package["name"], []):
        extra = base / relative
        if not extra.is_file():
            sys.exit(f"error: {package['name']} {package['version']}: expected licence file {relative} is missing")
        found[extra] = None
    if package.get("license_file"):
        explicit = base / package["license_file"]
        if explicit.is_file():
            found[explicit] = None
    return [normalise(path.read_bytes()) for path in found if path.stat().st_size]


def copyright_line(package):
    names = [re.sub(r"\s*<[^>]*>", "", author).strip() for author in package.get("authors") or []]
    names = [name for name in names if name]
    if names:
        return "Copyright (c) " + ", ".join(names)
    return "Copyright (c) the " + package["name"] + " authors (" + (package.get("repository") or "no repository listed") + ")"


# ---- document ---------------------------------------------------------------------------

def collect(selectors, packages):
    entries = []
    problems = []
    fallback = []
    for name, version in sorted(resolved_crates(selectors)):
        package = packages.get((name, version))
        if package is None:
            sys.exit(f"error: {name} {version} is in the dependency tree but not in cargo metadata")
        expression = package["license"] or EXPRESSION_OVERRIDES.get(name)
        if not expression:
            problems.append(f"{name} {version}: no SPDX licence expression in Cargo.toml (licence-file only?)")
            continue
        try:
            node = parse_expression(expression)
        except ValueError as error:
            problems.append(f"{name} {version}: {error}")
            continue
        chosen = choose(node)
        if chosen is None:
            problems.append(f"{name} {version}: licence `{expression}` is not on the allow-list")
            continue
        texts = licence_files(package)
        for identifier, line in EXTRA_SPDX.get(name, []):
            texts.append(line + "\n\n" + SPDX[identifier])
        used_fallback = not texts
        if used_fallback:
            fallback.append(f"{name} {version} ({expression})")
            for identifier, _ in chosen:
                if identifier not in SPDX:
                    problems.append(f"{name} {version}: ships no licence file and no SPDX text for {identifier} is built in")
                    continue
                texts.append(copyright_line(package) + "\n\n" + SPDX[identifier])
        entries.append((name, version, expression, texts, used_fallback))
    return entries, problems, fallback


SYSTEM_LIBRARIES = """
## System libraries linked in the container image

The `native-geospatial` binary links these libraries dynamically; they are not Rust crates.
GDAL and PROJ are built from the pinned sources in `Dockerfile`; the others are Debian trixie
packages, installed unmodified, whose copyright files are in `/usr/share/doc/<package>/copyright`
in the image.

| Library | Version | Licence | Licence text in the image |
|---|---|---|---|
| GDAL | 3.12.4 (source build) | MIT (X11), with bundled third-party sources under permissive licences | `/usr/share/doc/rusty-tiles/third_party/system/gdal/LICENSE.TXT` |
| PROJ | 9.8.1 (source build) | MIT | `/usr/share/doc/rusty-tiles/third_party/system/proj/COPYING` |
| GEOS (`libgeos-c1t64`) | Debian trixie | LGPL-2.1-or-later | `/usr/share/doc/libgeos-c1t64/copyright` |
| SQLite (`libsqlite3-0`) | Debian trixie | public domain | `/usr/share/doc/libsqlite3-0/copyright` |
| libtiff (`libtiff6`) | Debian trixie | libtiff (permissive, BSD-style) | `/usr/share/doc/libtiff6/copyright` |
| GNU C++ runtime (`libstdc++6`) | Debian trixie | GPL-3.0-or-later WITH GCC-exception-3.1 | `/usr/share/doc/libstdc++6/copyright` |

This table lists the main libraries rusty-tiles links; it is not an exhaustive list of the image's
Debian packages, each of which keeps its own copyright file under `/usr/share/doc/<package>/`.
GEOS (LGPL-2.1-or-later) is linked dynamically and unmodified, so it can be replaced by another
build of the same library; its source is available from Debian (`apt source geos`). The GNU C++
runtime is covered by the GCC Runtime Library Exception. The Rust crate list above is separate from
this table and contains no copyleft licence.
"""


BODY_MARKERS = ("Permission is hereby granted", "Redistribution and use in source and binary forms",
                "Permission to use, copy, modify", "This software is provided 'as-is'",
                "This is free and unencumbered software")
TITLE = re.compile(r"^(the )?(mit|bsd|isc|zlib)\b[^\n]*(license|licence)[^\n]*$|^bsd \d-clause license$", re.I)


def split_text(text):
    """Split a licence file into (notices, body) units. The body is the standard licence text
    and is written once however many crates use it; the notices are the copyright lines."""
    start = re.search(r"^\s*Apache License\s*$", text, re.M)
    end = re.search(r"END OF TERMS AND CONDITIONS[^\n]*", text)
    if start and end and not text[:start.start()].strip():
        tail = text[end.end():].strip()
        units = [("", text[start.start():end.end()])]
        if tail and not re.search(r"\[yyyy\]|\[name of copyright owner\]", tail):
            units.append(("", tail))  # custom notice after the terms; the template appendix is dropped
        return units
    positions = [i for i in (text.find(marker) for marker in BODY_MARKERS) if 0 <= i < 500]
    if positions:
        cut = min(positions)
        lines = [line.strip() for line in text[:cut].split("\n") if line.strip()]
        lines = [line for line in lines if not TITLE.match(line)]
        return [("; ".join(lines), text[cut:])]
    return [("", text)]


def render(artefact, entries):
    _, label = ARTEFACTS[artefact]
    bodies = {}  # whitespace-insensitive key -> [body, {notice: [crates]}]
    order = []
    index = {}
    for name, version, expression, crate_texts, _ in entries:
        numbers = []
        for text in crate_texts:
            for notice, body in split_text(text):
                key = hashlib.sha256(" ".join(body.split()).encode()).hexdigest()
                if key not in bodies:
                    bodies[key] = [body, {}]
                    order.append(key)
                bodies[key][1].setdefault(notice, []).append(f"{name} {version}")
                numbers.append(order.index(key) + 1)
        index[(name, version)] = sorted(set(numbers))
    counts = {}
    for _, _, expression, _, _ in entries:
        counts[expression] = counts.get(expression, 0) + 1
    lines = [
        f"# Third-party licences: rusty-tiles {label}",
        "",
        "Generated by `scripts/third_party_licenses.py`; do not edit. rusty-tiles itself is "
        "`MIT AND Apache-2.0 AND BSD-3-Clause` (see `LICENSE` and, for the vendored Basis Universal "
        "encoder, `third_party/basis_universal/`). This file lists the "
        f"{len(entries)} Rust crates in the resolved dependency graph of this artefact (normal and build "
        "dependencies, all target platforms) and the licence texts those crates publish. Where a crate "
        "publishes no licence file, the SPDX standard text and a copyright line from its manifest are given.",
        "",
    ]
    if artefact != "native-geospatial":
        lines += [
            "`libsqlite3-sys` compiles the SQLite amalgamation into this artefact; SQLite is in the "
            "public domain (https://www.sqlite.org/copyright.html).",
            "",
        ]
    lines += [
        "## Licences",
        "",
    ]
    lines += [f"- `{expression}`: {count}" for expression, count in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))]
    lines += ["", "## Crates", "", "Each crate is followed by its licence expression and the numbers of its texts below.", ""]
    lines += [f"- {name} {version}: {expression} [{', '.join(map(str, index[(name, version)]))}]"
              for name, version, expression, _, _ in entries]
    lines += ["", "## Licence texts", ""]
    for number, key in enumerate(order, 1):
        body, notices = bodies[key]
        fence = "`" * max(3, 1 + max((len(run) for run in re.findall(r"`+", body)), default=0))
        lines += [f"### [{number}]", ""]
        lines += [f"- {', '.join(users)}" + (f": {notice}" if notice else "")
                  for notice, users in sorted(notices.items(), key=lambda kv: kv[1][0])]
        lines += ["", fence, body, fence, ""]
    document = "\n".join(lines)
    if artefact == "native-geospatial":
        document = document.rstrip("\n") + "\n" + SYSTEM_LIBRARIES
    return document.rstrip("\n") + "\n"


def build(only=None):
    packages = metadata()
    failed = False
    outputs = {}
    for artefact, (selectors, _) in ARTEFACTS.items():
        if only and artefact not in only:
            continue
        entries, problems, fallback = collect(selectors, packages)
        for problem in problems:
            print(f"error: {artefact}: {problem}", file=sys.stderr)
            failed = True
        outputs[artefact] = render(artefact, entries) if not problems else None
        print(f"{artefact}: {len(entries)} crates, {len(fallback)} with SPDX fallback text")
        for item in fallback:
            print(f"  fallback: {item}")
    return outputs, failed


def path_for(artefact):
    return OUT / f"THIRD_PARTY_LICENSES-{artefact}.md"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("command", choices=["generate", "check"])
    args = parser.parse_args()
    outputs, failed = build()
    if failed:
        return 1
    if args.command == "generate":
        OUT.mkdir(exist_ok=True)
        for artefact, text in outputs.items():
            path_for(artefact).write_text(text, encoding="utf-8", newline="\n")
        return 0
    for artefact, text in outputs.items():
        path = path_for(artefact)
        if not path.is_file() or path.read_text(encoding="utf-8") != text:
            print(f"error: {path.relative_to(ROOT)} is out of date; run "
                  "`python3 scripts/third_party_licenses.py generate` and commit it", file=sys.stderr)
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
