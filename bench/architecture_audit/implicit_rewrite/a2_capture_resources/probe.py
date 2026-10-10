#!/usr/bin/env python3
"""Independent A2 admission/identity/accounting/lifecycle MODELS; no Rust execution.

Standard library only. Creates one new output directory. Never run with -O.
Classic stored ZIP is deliberately the model's narrow fixture envelope, not a
claim that ZIP64 is forbidden by A2 or that this is a 3TZ/index validator.
"""
import hashlib
import io
import json
import math
import os
from pathlib import Path
import resource
import struct
import sys
import tracemalloc
import zipfile

MIB = 1024 * 1024
U64 = (1 << 64) - 1


class Refusal(Exception):
    def __init__(self, kind, reason):
        self.kind, self.reason = kind, reason
        super().__init__(reason)


def refuse(kind, reason):
    raise Refusal(kind, reason)


def checked_add(a, b, ceiling, reason):
    if a < 0 or b < 0 or a > ceiling or b > ceiling - a:
        refuse("unsupported", reason)
    return a + b


def checked_mul(a, b, ceiling, reason):
    if a < 0 or b < 0 or (b and a > ceiling // b):
        refuse("unsupported", reason)
    return a * b


def signature(st):
    return st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns


def capture(source, output, ceiling=64 * MIB, hook=None):
    """Actual POSIX file operations for an independently authored capture model."""
    source, output = Path(os.path.abspath(source)), Path(os.path.abspath(output))
    if source.is_symlink():
        refuse("invalid_input", "source symlink leaf")
    canonical = source.resolve(strict=True)
    if output.is_symlink():
        refuse("invalid_request", "output symlink leaf")
    destination = output.resolve(strict=False)
    # A destination below a selected regular source is invalid even if absent.
    if destination == canonical or canonical in destination.parents:
        refuse("invalid_request", "path overlap")
    fd = os.open(canonical, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        import stat
        if not stat.S_ISREG(before.st_mode):
            refuse("invalid_input", "source is not regular")
        if before.st_size > ceiling:
            refuse("unsupported", "source archive ceiling before capture")
        if signature(before) != signature(os.stat(source)):
            refuse("invalid_input", "source changed before capture")
        if output.exists():
            out = os.stat(output)
            if (out.st_dev, out.st_ino) == (before.st_dev, before.st_ino):
                refuse("invalid_request", "native identity overlap")
        parts, count = [], 0
        while True:
            portion = os.read(fd, min(65536, before.st_size + 1 - count))
            if not portion:
                break
            count += len(portion)
            if count > before.st_size:
                refuse("invalid_input", "capture grew")
            parts.append(portion)
            if hook:
                hook()
                hook = None
        if count != before.st_size or signature(before) != signature(os.fstat(fd)):
            refuse("invalid_input", "capture metadata changed")
        try:
            now = os.stat(source)
            stable_path = not source.is_symlink() and source.resolve(strict=True) == canonical
        except FileNotFoundError:
            refuse("invalid_input", "source disappeared during capture")
        if not stable_path or signature(before) != signature(now):
            refuse("invalid_input", "capture pathname changed")
        return b"".join(parts), destination
    finally:
        os.close(fd)


def inventory(names, collision_kind="invalid_input"):
    """Complete file inventory; a sorted predecessor cannot catch every ancestor."""
    files, parents = set(), set()
    for name in names:
        bits = name.split("/")
        if (not name or any(p in ("", ".", "..") for p in bits)
                or any(c in name for c in "\\:\x00")):
            refuse("invalid_input", "unsafe archive member")
        ancestors = {"/".join(bits[:n]) for n in range(1, len(bits))}
        if name in files or name in parents or ancestors & files:
            refuse(collision_kind, "exact or ancestor collision")
        files.add(name)
        parents.update(ancestors)
    return files


def admit_zip(blob, member_limit=64 * MIB, aggregate_limit=64 * MIB,
              entries_limit=16384, directory_limit=2 * MIB):
    """No ZipFile/entry list/payload allocation before complete envelope checks.

    Metadata allocations are bounded per field and by directory/count ceilings.
    Does not implement ZIP64, descriptors, CRC/index validation or general ZIP.
    """
    if len(blob) > 64 * MIB:
        refuse("unsupported", "archive byte ceiling")
    tail_start = max(0, len(blob) - 65557)
    end = next((p for p in range(len(blob) - 22, tail_start - 1, -1)
                if blob[p:p + 4] == b"PK\x05\x06"
                and p + 22 + struct.unpack_from("<H", blob, p + 20)[0] == len(blob)), None)
    if end is None:
        refuse("invalid_input", "missing end record")
    disk, directory_disk, local_count, count, size, offset, _ = struct.unpack_from("<4H2IH", blob, end + 4)
    if disk or directory_disk:
        refuse("unsupported", "multi-disk")
    if count == 65535 or size == 0xffffffff or offset == 0xffffffff:
        refuse("unsupported", "model omits ZIP64")
    if local_count != count:
        refuse("invalid_input", "count mismatch")
    if count > entries_limit or size > directory_limit:
        refuse("unsupported", "directory/count ceiling")
    if offset + size != end:
        refuse("invalid_input", "directory range")
    cursor, total, names, ranges = offset, 0, [], []
    for _ in range(count):
        if cursor + 46 > end or blob[cursor:cursor + 4] != b"PK\x01\x02":
            refuse("invalid_input", "central entry range/signature")
        flags, method = struct.unpack_from("<HH", blob, cursor + 8)
        compressed, decoded = struct.unpack_from("<II", blob, cursor + 20)
        nl, el, cl = struct.unpack_from("<HHH", blob, cursor + 28)
        local = struct.unpack_from("<I", blob, cursor + 42)[0]
        following = cursor + 46 + nl + el + cl
        if following > end:
            refuse("invalid_input", "central fields range")
        if flags & (1 | 8):
            refuse("unsupported", "model omits encryption/descriptors")
        if method != 0:
            refuse("invalid_input", "3TZ requires stored members; no decompression")
        if compressed != decoded:
            refuse("invalid_input", "stored sizes differ")
        if decoded > member_limit:
            refuse("unsupported", "member ceiling before payload read")
        total = checked_add(total, decoded, aggregate_limit, "aggregate member ceiling")
        if local + 30 > offset or blob[local:local + 4] != b"PK\x03\x04":
            refuse("invalid_input", "local header range/signature")
        lflags, lmethod = struct.unpack_from("<HH", blob, local + 6)
        lcompressed, ldecoded = struct.unpack_from("<II", blob, local + 18)
        lnl, lel = struct.unpack_from("<HH", blob, local + 26)
        data = local + 30 + lnl + lel
        if data + decoded > offset:
            refuse("invalid_input", "payload range")
        raw = blob[cursor + 46:cursor + 46 + nl]
        if ((flags, method, compressed, decoded, nl) !=
                (lflags, lmethod, lcompressed, ldecoded, lnl)
                or raw != blob[local + 30:local + 30 + lnl]):
            refuse("invalid_input", "local/central mismatch")
        try:
            names.append(raw.decode("utf-8"))
        except UnicodeDecodeError:
            refuse("unsupported", "non-UTF8 archive member")
        ranges.append((local, data + decoded))
        cursor = following
    if cursor != end:
        refuse("invalid_input", "directory count/length mismatch")
    inventory(names)
    ranges.sort()
    if any(a[1] > b[0] for a, b in zip(ranges, ranges[1:])):
        refuse("invalid_input", "overlapping member storage")
    # The payload owner is blob. Production should use ranges, not read copies.
    return {"entries": count, "member_bytes": total, "directory_bytes": size}


def parse_json(blob, max_bytes=1024 * 1024, max_depth=31, max_items=65536):
    """A simple independent bounded JSON parser: count before object construction."""
    if len(blob) > max_bytes:
        refuse("unsupported", "JSON byte ceiling")
    text, position, items = blob.decode("utf-8"), 0, 0
    decoder = json.JSONDecoder()

    def whitespace():
        nonlocal position
        while position < len(text) and text[position] in " \t\r\n":
            position += 1

    def string():
        nonlocal position
        if position >= len(text) or text[position] != '"':
            refuse("invalid_input", "object key is not a string")
        try:
            value, end = decoder.raw_decode(text, position)
        except ValueError:
            refuse("invalid_input", "invalid JSON string")
        if not isinstance(value, str):
            refuse("invalid_input", "object key is not a string")
        position = end
        return value

    def value(depth):
        nonlocal position, items
        whitespace()
        if depth > max_depth or items >= max_items:
            refuse("unsupported", "JSON depth/items ceiling")
        items += 1
        if position >= len(text):
            refuse("invalid_input", "missing JSON value")
        opening = text[position]
        if opening not in "[{":
            try:
                result, position = decoder.raw_decode(text, position)
            except ValueError:
                refuse("invalid_input", "invalid JSON scalar")
            if isinstance(result, float) and not math.isfinite(result):
                refuse("invalid_input", "nonfinite JSON scalar")
            return result
        position += 1
        closing, result = ("}", {}) if opening == "{" else ("]", [])
        whitespace()
        if position < len(text) and text[position] == closing:
            position += 1
            return result
        while True:
            whitespace()
            if opening == "{":
                key = string()
                if key in result:
                    refuse("invalid_input", "duplicate JSON key")
                whitespace()
                if position >= len(text) or text[position] != ":":
                    refuse("invalid_input", "missing JSON colon")
                position += 1
                result[key] = value(depth + 1)
            else:
                result.append(value(depth + 1))
            whitespace()
            if position >= len(text):
                refuse("invalid_input", "unterminated JSON container")
            delimiter = text[position]
            position += 1
            if delimiter == closing:
                return result
            if delimiter != ",":
                refuse("invalid_input", "invalid JSON delimiter")
    result = value(0)
    whitespace()
    if position != len(text):
        refuse("invalid_input", "trailing JSON")
    return result, items


def output_bound(originals, aliases, documents, report, names):
    # Current codec exact envelope below classic ZIP thresholds, no extras/comment.
    # `names` excludes its index, which is emitted exactly once and indexed only
    # for other members. Source index is discarded rather than copied.
    index = checked_mul(len(names), 24, U64, "index overflow")
    member_bytes = 0
    for n in (originals, aliases, documents, report, index):
        member_bytes = checked_add(member_bytes, n, U64, "member bytes overflow")
    envelope = 22
    for name in [*names, "@3dtilesIndex1@"]:
        envelope = checked_add(envelope, 76 + 2 * len(name.encode()), U64, "ZIP overhead overflow")
    return checked_add(member_bytes, envelope, 160 * MIB, "final archive ceiling")


STAGES = ("capture", "admit", "plan", "observer", "workspace", "payload",
          "report", "archive", "inspect_final", "cleanup", "ready", "close",
          "seal", "publish")


def lifecycle(fault=None, concurrent_cause=None, cleanup_fault=False):
    """Single serial attempt schedule. No real runtime/fault injection is claimed."""
    trace, primary, secondary, retained, committed = [], None, [], [], False
    workspace = False
    for stage in STAGES:
        trace.append(stage)
        workspace |= stage == "workspace"
        if stage == fault:
            primary = concurrent_cause or ("conflict" if stage == "publish" else "io")
            if concurrent_cause:
                secondary.append("io")
            if workspace and cleanup_fault:
                secondary.append("cleanup_io")
                retained.append("owned-workspace")
            return dict(primary=primary, secondary=secondary, retained=retained,
                        committed=False, trace=trace)
        if stage == "cleanup":
            if cleanup_fault:
                return dict(primary="io", secondary=[], retained=["owned-workspace"],
                            committed=False, trace=trace)
            workspace = False
        if stage == "publish":
            committed = True
    return dict(primary=primary, secondary=secondary, retained=retained,
                committed=committed, trace=trace, cleanup_diagnostics=[])


def make_zip(members):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, payload in members:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            archive.writestr(info, payload)
    return buf.getvalue()


def main():
    if len(sys.argv) != 2 or not __debug__:
        raise SystemExit("usage: python3 -B probe.py /new/output/directory (without -O)")
    output = Path(sys.argv[1]).absolute()
    output.mkdir(parents=True, exist_ok=False)
    rows = []

    def rejected(label, operation, kind):
        try:
            operation()
        except Refusal as failure:
            assert failure.kind == kind, (label, failure.kind, kind)
            rows.append(dict(case=label, outcome="refused", kind=failure.kind, reason=failure.reason))
        else:
            raise AssertionError(label + " unexpectedly admitted")

    blob = make_zip([("tileset.json", b'{"root":{}}'), ("nested/payload.glb", b"payload")])
    facts = admit_zip(blob)
    source = output / "source.3tz"
    source.write_bytes(blob)
    captured, destination = capture(source, output / "result.3tz")
    assert captured == blob
    rows.append(dict(case="bounded_classic_stored_zip", outcome="admitted", **facts))
    rejected("source_bytes_before_capture", lambda: capture(source, destination, ceiling=8), "unsupported")
    rejected("same_path", lambda: capture(source, source), "invalid_request")
    hardlink = output / "hardlink.3tz"
    os.link(source, hardlink)
    rejected("hardlink_output", lambda: capture(source, hardlink), "invalid_request")
    symlink = output / "symlink.3tz"
    symlink.symlink_to(source)
    rejected("symlink_output_leaf", lambda: capture(source, symlink), "invalid_request")
    rejected("symlink_source_leaf", lambda: capture(symlink, destination), "invalid_input")
    parent_alias = output / "parent_alias"
    parent_alias.symlink_to(output, target_is_directory=True)
    rejected("symlink_parent_same_path", lambda: capture(source, parent_alias / source.name), "invalid_request")
    rejected("absent_descendant_of_source", lambda: capture(source, source / "out.3tz"), "invalid_request")

    def replace_during_capture():
        source.unlink()
        source.write_bytes(blob)
    rejected("identity_replacement_during_capture", lambda: capture(source, destination, hook=replace_during_capture), "invalid_input")
    def grow_during_capture():
        with source.open("ab") as stream:
            stream.write(b"!")
    rejected("growth_during_capture", lambda: capture(source, destination, hook=grow_during_capture), "invalid_input")
    source.write_bytes(blob)
    def shorten_during_capture():
        source.write_bytes(b"short")
    rejected("shrink_during_capture", lambda: capture(source, destination, hook=shorten_during_capture), "invalid_input")
    source.write_bytes(blob)
    prepared, absolute_destination = capture(source, destination)
    previous_cwd = Path.cwd()
    try:
        os.chdir(output)
        source.unlink()
        source.write_bytes(b"callback replacement")
        destination.write_bytes(b"callback destination")
        assert prepared == blob and absolute_destination == destination
        assert destination.read_bytes() == b"callback destination"
    finally:
        os.chdir(previous_cwd)
    rows.append(dict(case="post_capture_source_cwd_destination_mutation", outcome="captured_bytes_and_absolute_output_unchanged", publication_race="must be resolved by F0 CreateNew/Replace; not executed here"))
    # Sensitive proof-limit control: metadata rechecks are not an atomic snapshot.
    unstable = output / "unstable.3tz"
    unstable.write_bytes(b"A" * 131072)
    original_stat = unstable.stat()
    def replace_second_half_restore_metadata():
        with unstable.open("r+b") as stream:
            stream.seek(65536)
            stream.write(b"B" * 65536)
        os.utime(unstable, ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns))
    mixed, _ = capture(unstable, output / "unstable-result.3tz", hook=replace_second_half_restore_metadata)
    assert mixed == b"A" * 65536 + b"B" * 65536
    rows.append(dict(case="metadata_restored_in_place_change", outcome="model_admits_mixed_bytes", implication="stable source precondition required; identity/length/mtime cannot prove atomic snapshot"))
    bad = bytearray(blob)
    end = len(bad) - 22
    struct.pack_into("<HH", bad, end + 8, 20000, 20000)
    rejected("entry_count_before_directory", lambda: admit_zip(bad), "unsupported")
    bad = bytearray(blob)
    struct.pack_into("<I", bad, end + 12, 3 * MIB)
    rejected("directory_bytes_before_directory", lambda: admit_zip(bad), "unsupported")
    central = blob.index(b"PK\x01\x02")
    bad = bytearray(blob)
    struct.pack_into("<II", bad, central + 20, 100 * MIB, 100 * MIB)
    rejected("member_bytes_before_payload", lambda: admit_zip(bad), "unsupported")
    bad = bytearray(blob)
    struct.pack_into("<H", bad, central + 10, 8)
    rejected("deflate_without_decompression", lambda: admit_zip(bad), "invalid_input")
    bad = bytearray(blob)
    struct.pack_into("<I", bad, central + 42, 0xffffffff)
    rejected("malformed_local_offset", lambda: admit_zip(bad), "invalid_input")
    rejected("aggregate_stored_bytes", lambda: admit_zip(blob, aggregate_limit=12), "unsupported")
    rejected("truncated_zip", lambda: admit_zip(blob[:-1]), "invalid_input")
    names = ["nested/payload.glb", "nested/owned-0-0-0.glb", "tileset.json", "implicit-0.json", "subtrees/0.subtree", "conversion.json", "@3dtilesIndex1@"]
    assert len(inventory(names)) == len(names)
    for label, colliding in [
        ("source_subtrees_ancestor", ["subtrees", "subtrees/0.subtree"]),
        ("alias_exact", ["nested/a.glb", "nested/a.glb"]),
        ("alias_ancestor_forward", ["nested/alias", "nested/alias/0.glb"]),
        ("alias_ancestor_reverse", ["nested/alias/0.glb", "nested/alias"]),
        ("generated_document_ancestor", ["implicit-0.json/resource", "implicit-0.json"]),
        ("report_exact", ["conversion.json", "conversion.json"]),
        ("index_exact", ["@3dtilesIndex1@", "@3dtilesIndex1@"]),
        ("ancestor_with_intervening_sorted_name", ["a", "a-", "a/x"]),
    ]:
        rejected(label, lambda names=colliding: inventory(names, collision_kind="unsupported"), "unsupported")
    assert parse_json(b'{"a":[1,2],"b":true}')[1] == 5
    for label, text, kwargs, kind in [
        ("JSON_duplicate_spoof", b'{"resource limit:":1,"resource limit:":2}', {}, "invalid_input"),
        ("JSON_depth_before_tree", b"[" * 33 + b"0" + b"]" * 33, {}, "unsupported"),
        ("JSON_items_before_next_value", b"[0,0,0,0]", {"max_items": 4}, "unsupported"),
        ("JSON_bytes_before_decode", b"{} ", {"max_bytes": 2}, "unsupported"),
        ("JSON_trailing", b"{} {}", {}, "invalid_input"),
        ("JSON_nonfinite", b"1e999", {}, "invalid_input"),
        ("JSON_invalid_container_key", b"{{}:1}", {}, "invalid_input"),
    ]:
        rejected(label, lambda text=text, kwargs=kwargs: parse_json(text, **kwargs), kind)
    aliases = checked_mul(4096, 16384, 64 * MIB, "alias ceiling")
    assert aliases == 64 * MIB
    rejected("shared_payload_alias_amplification", lambda: checked_mul(4097, 16384, 64 * MIB, "alias ceiling"), "unsupported")
    rejected("repeated_header_amplification", lambda: checked_mul(4096, 65536, 16 * MIB, "generated document ceiling"), "unsupported")
    rejected("u64_count_overflow", lambda: checked_mul(U64, 24, U64, "index overflow"), "unsupported")
    bound = output_bound(64 * MIB, 64 * MIB, 16 * MIB, 65536, [f"x/{n}" for n in range(16383)])
    rows.append(dict(case="checked_final_bound", outcome="bounded", final_bytes=bound, ceiling=160 * MIB, includes="originals aliases documents report index local+central headers names EOCD"))
    for stage in STAGES:
        failure = lifecycle(fault=stage)
        assert not failure["committed"] and failure["trace"][-1] == stage
        if STAGES.index(stage) < STAGES.index("workspace"):
            assert "workspace" not in failure["trace"]
    result = lifecycle(fault="archive", concurrent_cause="cancelled", cleanup_fault=True)
    assert result["primary"] == "cancelled" and result["secondary"] == ["io", "cleanup_io"] and result["retained"]
    result = lifecycle(fault="observer", concurrent_cause="observer_failure")
    assert result["primary"] == "observer_failure" and "workspace" not in result["trace"]
    result = lifecycle(cleanup_fault=True)
    assert result["primary"] == "io" and "ready" not in result["trace"]
    successful = lifecycle()
    assert successful["committed"] and successful["trace"].count("archive") == successful["trace"].count("inspect_final") == 1
    assert successful["trace"][-1] == "publish"
    rows.append(dict(case="serial_lifecycle_schedule", outcome="model_pass", faulted_stages=len(STAGES), first_cause_preserved=True, cleanup_before_ready=True, no_postcommit_domain_work=True))
    tracemalloc.start()
    documents = [json.dumps({"header": "x" * 1000, "root": {"id": n}}).encode() for n in range(4096)]
    serialized_bytes = sum(map(len, documents))
    current, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    rows.append(dict(case="small_repeated_header_python_measurement", outcome="measured_model_only", documents=len(documents), serialized_bytes=serialized_bytes, tracemalloc_live_bytes=current, tracemalloc_peak_bytes=peak))
    receipt = dict(evidence="independent Python models, including actual POSIX identity/capture operations; no Rust/CLI producer execution", python=sys.version, platform=sys.platform, pid=os.getpid(), driver_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), fixture_sha256=hashlib.sha256(blob).hexdigest(), cases=rows, rss_high_water_kib_linux=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss, open_fds_at_end=len(list(Path('/proc/self/fd').iterdir())) if Path('/proc/self/fd').exists() else None, limits="proposal only; no Rust RSS/descriptor/scratch acceptance")
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"cases": len(rows), "receipt": str(output / "receipt.json"), "driver_sha256": receipt["driver_sha256"]}))


if __name__ == "__main__":
    main()
