#!/usr/bin/env python3
"""Compare published literal golden bytes with retained fixture; never decode."""
import hashlib
import io
import json
from pathlib import Path
import re
import struct
import zipfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[4]


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


source = (HERE / "meshopt_decoder.test.js").read_text()
section = source.split("decodeVertexBuffer: function () {", 1)[1].split("decodeVertexBuffer_More:", 1)[0]


def literal_array(name):
    literal = re.search(r"var " + name + r" = new Uint8Array\(\[([\s\S]*?)\]\);", section).group(1)
    return bytes(int(token.strip(), 0) for token in literal.split(",") if token.strip())


encoded = literal_array("encoded")
expected = literal_array("expected")
assert len(encoded) == 85 and len(expected) == 48
assert "decoder.decodeVertexBuffer(result, 4, 12, encoded);" in section
(HERE / "golden-compressed.bin").write_bytes(encoded)
(HERE / "golden-expected.bin").write_bytes(expected)

fixture = REPO / "tests/fixtures/c1/meshopt_none_golden.3tz"
archive = fixture.read_bytes()
with zipfile.ZipFile(io.BytesIO(archive)) as source_archive:
    assert source_archive.testzip() is None
    content_name, = [name for name in source_archive.namelist() if name.endswith(".glb")]
    glb = source_archive.read(content_name)
magic, version, length = struct.unpack_from("<4sII", glb)
assert (magic, version, length) == (b"glTF", 2, len(glb))
json_length, json_tag = struct.unpack_from("<II", glb, 12)
assert json_tag == 0x4E4F534A
doc = json.loads(glb[20:20 + json_length])
binary_length, binary_tag = struct.unpack_from("<II", glb, 20 + json_length)
assert binary_tag == 0x004E4942
binary = glb[28 + json_length:28 + json_length + binary_length]
ext = doc["bufferViews"][0]["extensions"]["EXT_meshopt_compression"]
assert doc["buffers"][0]["byteLength"] == len(encoded)
assert binary[:len(encoded)] == encoded
assert (ext["count"], ext["byteStride"], ext["mode"], ext.get("filter", "NONE")) == (4, 12, "ATTRIBUTES", "NONE")
assert ext["byteLength"] == len(encoded) and ext.get("byteOffset", 0) == 0

# The old generator is read only for byte-identity comparison, never imported
# or run. Published expected bytes remain the reference truth.
oracle = REPO / "tests/c1_validation_oracle.py"
oracle_text = oracle.read_text()
old_encoded, = re.findall(r"encoded = bytes.fromhex\('([0-9a-f]+)'\)", oracle_text)
old_expected, = re.findall(r"decoded = bytes.fromhex\('([0-9a-f]+)'\)", oracle_text)
assert bytes.fromhex(old_encoded) == encoded
assert bytes.fromhex(old_expected) == expected

record = {
    "schemaVersion": 1,
    "scope": "Static independent upstream literal extraction and retained-byte comparison; no codec execution or decoder acceptance.",
    "upstreamSourceSha256": sha((HERE / "meshopt_decoder.test.js").read_bytes()),
    "upstreamTest": "decodeVertexBuffer",
    "compressed": {"bytes": len(encoded), "sha256": sha(encoded), "headerByte": encoded[0]},
    "publishedExpected": {"bytes": len(expected), "sha256": sha(expected)},
    "count": 4, "stride": 12, "mode": "ATTRIBUTES", "filter": "NONE",
    "retainedFixture": {"path": str(fixture), "sha256": sha(archive), "contentName": content_name, "contentSha256": sha(glb), "compressedPrefixMatches": True},
    "retainedGeneratorLiteralComparison": {"path": str(oracle), "sha256": sha(oracle.read_bytes()), "compressedMatches": True, "expectedMatches": True},
    "driverSha256": sha(Path(__file__).read_bytes()),
}
(HERE / "golden-byte-comparison.json").write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
print(json.dumps(record, sort_keys=True))
