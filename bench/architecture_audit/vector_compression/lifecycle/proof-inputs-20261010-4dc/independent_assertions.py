"""UNEXECUTED narrow oracle helpers for coordinator-owned prepared probes.

No production imports or decoder implementation. Decoded physical view bytes
must come from an independently pinned external decoder invocation, not C1.
Permission checks prove observed state, never normative codec validity/CAS.
"""
import hashlib
import json
import os
import stat
import struct
from decimal import Decimal
from pathlib import Path

OPAQUE = b'{"integer":18446744073709551615,"exponent":1e400,"negativeZero":-0.0}'


def file_facts(path):
    path = Path(path)
    entry = path.lstat()
    assert stat.S_ISREG(entry.st_mode), "expected a regular non-symlink leaf"
    return {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "bytes": entry.st_size, "dev": entry.st_dev, "ino": entry.st_ino,
            "mtime_ns": entry.st_mtime_ns, "mode": stat.S_IMODE(entry.st_mode),
            "readonly": bool(getattr(entry, "st_file_attributes", 0) & 1) if os.name == "nt" else not bool(entry.st_mode & 0o222)}


def assert_unchanged_content_identity(before, after):
    for key in ["sha256", "bytes", "dev", "ino"]:
        assert before[key] == after[key], (key, before, after)


def assert_callback_permission_conflict(original, final, observed_kind, events, temporary_paths):
    # A chmod performed deliberately by the test observer is an external edit.
    # Failure must preserve its final permission state, not undo the callback.
    assert_unchanged_content_identity(original, final)
    assert final["mode"] != original["mode"] or final["readonly"] != original["readonly"]
    assert observed_kind == "conflict"
    assert "install" not in events
    assert not temporary_paths, "ordinary cleanup was expected; injected cleanup tests are separate"


def assert_windows_readonly_refusal(before, after, observed_kind, events, temporary_paths):
    assert before["readonly"]
    assert_unchanged_content_identity(before, after)
    assert after["readonly"] == before["readonly"]
    assert after["mode"] == before["mode"]
    assert observed_kind == "unsupported"
    assert not any(e.startswith("observer") or e in {"stage_create", "encode", "install"} for e in events), "no observer/producer/staging/install calls allowed"
    assert not temporary_paths


def assert_named_entry_replacement(before_source, after_source, before_alias, after_alias):
    assert_unchanged_content_identity(before_alias, after_alias)
    assert before_source["sha256"] == before_alias["sha256"]
    assert (before_source["dev"], before_source["ino"]) == (before_alias["dev"], before_alias["ino"])
    assert (after_source["dev"], after_source["ino"]) != (after_alias["dev"], after_alias["ino"])


def assert_same_run_fatal_resource(observed_kind, events, before_output, after_output, rejected):
    assert observed_kind == "resource_limit"
    assert "install" not in events
    assert not rejected, "resource refusal must not become FeatureRejected or skipped diagnostics"
    if before_output is None:
        assert after_output is None
    else:
        assert_unchanged_content_identity(before_output, after_output)
    assert events.count("attempt_begin") == 1


def envelope(data):
    assert len(data) >= 28
    magic, version, total = struct.unpack_from("<4sII", data, 0)
    assert (magic, version, total) == (b"glTF", 2, len(data))
    j, jt = struct.unpack_from("<I4s", data, 12)
    assert jt == b"JSON" and j % 4 == 0
    end = 20 + j
    u, ut = struct.unpack_from("<I4s", data, end)
    assert ut == b"BIN\0" and u % 4 == 0 and end + 8 + u == len(data)
    raw = data[20:end]
    doc = json.loads(raw, parse_int=int, parse_float=Decimal)
    return doc, raw, data[end + 8:], end, end + 8


def assert_selected_framing(data, declared_metadata):
    doc, raw, binary, json_end, bin_origin = envelope(data)
    embedded_length = doc["buffers"][0]["byteLength"]
    slack = len(binary) - embedded_length
    assert 0 <= slack <= (7 if declared_metadata else 3)
    assert not any(binary[embedded_length:])
    if declared_metadata:
        assert json_end % 8 == bin_origin % 8 == len(data) % 8 == 0
    return doc


def assert_preserved_views(record, output_bytes, independently_decoded_views):
    doc, raw, binary, _, _ = envelope(output_bytes)
    assert raw.count(OPAQUE) == 1, "opaque raw numeric subtree changed"
    for expected in record["expected_source_views"]:
        n = expected["view"]
        view = doc["bufferViews"][n]
        if "EXT_meshopt_compression" in view.get("extensions", {}):
            actual = independently_decoded_views[n]
        else:
            assert view.get("buffer", 0) == 0
            offset = view.get("byteOffset", 0)
            actual = binary[offset:offset + view["byteLength"]]
        assert actual == bytes.fromhex(expected["hex"]), (n, expected["role"])


def assert_identity(source_bytes, candidate_bytes):
    assert source_bytes == candidate_bytes, "all-raw/existing-compressed identity changed original bytes"
