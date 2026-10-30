"""Independent oracle for validator slice VS-C (limits surface).

Authored from docs/architecture/validator-limits-amendment.md and the ZIP/ZIP64
format only; it has not read src/, tests/ or bindings/.  Python 3 stdlib only,
runs under ``python3 -I``.
"""
import argparse
import concurrent.futures
import io
import json
import os
import struct
import subprocess
import sys

U64_MAX = 2 ** 64 - 1
MODEL = 1

CLASS_A = {
    "jsonBytes": 8388608,
    "jsonDepth": 64,
    "memberBytes": 134217728,
    "accessorElements": 4000000,
    "documentDecodedBytes": 134217728,
    "documentItems": 131072,
    "hierarchyDepth": 128,
    "containmentSpanBits": 4096,
    "containmentNodeDepth": 64,
    "liveDocumentBytes": 268435456,
}
CLASS_B = ["sourceArchiveBytes", "archiveEntries", "centralDirectoryBytes", "archiveStoredBytes"]
# key -> (floor, p, q)
CLASS_C = {
    "hierarchyVisits": (262144, 1, 8),
    "references": (1048576, 1, 8),
    "totalBytesRead": (2147483648, 4, 1),
    "totalPayloadElements": (16000000, 77, 1),
    "containmentWorkUnits": (536870912, 128, 1),
    "containmentBytesRead": (2147483648, 4, 1),
    "containmentDecodedBytes": (2147483648, 16, 1),
}
# "first S above the floor" column of the amendment's table
CROSSOVER = {
    "hierarchyVisits": 2097160,
    "references": 8388616,
    "totalBytesRead": 536870913,
    "totalPayloadElements": 207793,
    "containmentWorkUnits": 4194305,
    "containmentBytesRead": 536870913,
    "containmentDecodedBytes": 134217729,
}
OVERRIDABLE = set(CLASS_B) | set(CLASS_C)
ALL_KEYS = set(CLASS_A) | OVERRIDABLE


# --------------------------------------------------------------------------- expected

def expected(S, overrides=None):
    """Return (limits, limitDerivation) for stored-byte total S and overrides."""
    overrides = dict(overrides or {})
    for k, v in overrides.items():
        if k not in OVERRIDABLE:
            raise ValueError("key not overridable: %r" % (k,))
        if isinstance(v, bool) or not isinstance(v, int) or v < 0 or v > U64_MAX:
            raise ValueError("override not a u64: %r=%r" % (k, v))
    if not (0 <= S <= U64_MAX):
        raise ValueError("S out of u64 range")
    limits = dict(CLASS_A)
    for k in CLASS_B:
        limits[k] = overrides.get(k)
    budgets = {}
    for k, (floor, p, q) in CLASS_C.items():
        scaled = min(S * p // q, U64_MAX)
        if k in overrides:
            limits[k] = overrides[k]
            src = "explicit"
        else:
            limits[k] = max(floor, scaled)
            src = "scaled" if scaled > floor else "floor"
        budgets[k] = {"floor": floor, "perStoredByte": [p, q], "source": src}
    assert len(limits) == 21
    return limits, {"model": MODEL, "budgets": budgets}


def derivation_label(key, deriv, overrides):
    if key in CLASS_B:
        return "cap"
    return deriv["budgets"][key]["source"]


# --------------------------------------------------------------------------- parse

class ZipError(Exception):
    pass


def _open(path_or_obj):
    if isinstance(path_or_obj, (bytes, bytearray)):
        return io.BytesIO(bytes(path_or_obj)), True
    if hasattr(path_or_obj, "read"):
        return path_or_obj, False
    return open(path_or_obj, "rb"), True


def parse(path):
    """Return (N, S) from a ZIP/ZIP64 central directory; member data is not read."""
    f, close = _open(path)
    try:
        f.seek(0, 2)
        size = f.tell()
        tail_len = min(size, 22 + 65535)
        f.seek(size - tail_len)
        tail = f.read(tail_len)
        eocd = None
        i = len(tail) - 22
        while i >= 0:
            if tail[i:i + 4] == b"PK\x05\x06":
                clen = struct.unpack_from("<H", tail, i + 20)[0]
                if i + 22 + clen == len(tail):
                    eocd = i
                    break
            i -= 1
        if eocd is None:
            raise ZipError("no end of central directory")
        (_s, disk, cddisk, n_disk, n_total, cdsize, cdoff, _cl) = struct.unpack_from(
            "<IHHHHIIH", tail, eocd)
        eocd_abs = size - tail_len + eocd
        need64 = (n_total == 0xFFFF or n_disk == 0xFFFF or cdsize == 0xFFFFFFFF
                  or cdoff == 0xFFFFFFFF)
        loc_abs = eocd_abs - 20
        have_loc = False
        if loc_abs >= 0:
            f.seek(loc_abs)
            loc = f.read(20)
            have_loc = len(loc) == 20 and loc[:4] == b"PK\x06\x07"
        if have_loc or need64:
            if not have_loc:
                raise ZipError("ZIP64 fields required but no ZIP64 locator")
            _sig, _d, z64off, _tot = struct.unpack("<IIQI", loc)
            f.seek(z64off)
            rec = f.read(56)
            if len(rec) < 56 or rec[:4] != b"PK\x06\x06":
                raise ZipError("bad ZIP64 end record")
            (_sig, _rs, _vm, _vn, _d1, _d2, z_disk_n, z_total, z_cdsize, z_cdoff) = struct.unpack(
                "<IQHHIIQQQQ", rec)
            n_total, cdsize, cdoff = z_total, z_cdsize, z_cdoff
        f.seek(cdoff)
        cd = f.read(cdsize)
        if len(cd) != cdsize:
            raise ZipError("short central directory")
        pos = 0
        S = 0
        count = 0
        while count < n_total:
            if pos + 46 > len(cd) or cd[pos:pos + 4] != b"PK\x01\x02":
                raise ZipError("bad central header %d" % count)
            (_sg, _vm, _vn, _fl, method, _t, _d, _crc, csize, usize, nlen, elen, clen,
             _dn, _ia, _ea, loff) = struct.unpack_from("<IHHHHHHIIIHHHHHII", cd, pos)
            extra = cd[pos + 46 + nlen: pos + 46 + nlen + elen]
            e = 0
            z64 = None
            while e + 4 <= len(extra):
                hid, hsz = struct.unpack_from("<HH", extra, e)
                if hid == 1:
                    z64 = extra[e + 4: e + 4 + hsz]
                e += 4 + hsz
            zp = 0
            if usize == 0xFFFFFFFF:
                if z64 is None or zp + 8 > len(z64):
                    raise ZipError("missing ZIP64 uncompressed size")
                usize = struct.unpack_from("<Q", z64, zp)[0]
                zp += 8
            if csize == 0xFFFFFFFF:
                if z64 is None or zp + 8 > len(z64):
                    raise ZipError("missing ZIP64 compressed size")
                csize = struct.unpack_from("<Q", z64, zp)[0]
                zp += 8
            if method != 0 or csize != usize:
                raise ZipError("member %d is not stored (method=%d c=%d u=%d)" % (
                    count, method, csize, usize))
            S += csize
            pos += 46 + nlen + elen + clen
            count += 1
        return count, S
    finally:
        if close:
            f.close()


# --------------------------------------------------------------------------- check

def _find(obj, key):
    if isinstance(obj, dict):
        if key in obj:
            return obj[key], True
        for v in obj.values():
            if isinstance(v, dict) and key in v:
                return v[key], True
    return None, False


def _parse_json(text):
    text = text.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except ValueError:
        for line in reversed(text.splitlines()):
            try:
                return json.loads(line)
            except ValueError:
                continue
    return None


def check_one(binary, archive, overrides, timeout):
    row = {"archive": archive}
    problems = []
    cmd = [binary, "--json", "validate", archive]
    for k, v in overrides:
        cmd += ["--maxVolume", "%s=%d" % (k, v)]
    ov = {k: v for k, v in overrides}
    try:
        N, S = parse(archive)
        row["N"], row["S"] = N, S
    except Exception as exc:  # oracle could not parse
        N = S = None
        row["oracleParseError"] = str(exc)
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        row["exit"] = "timeout"
        row["disagreements"] = ["timeout"]
        return row, False, True
    row["exit"] = proc.returncode
    doc = _parse_json(proc.stdout)
    if doc is None:
        doc = _parse_json(proc.stderr)
    compared = False
    if proc.returncode == 0:
        if not isinstance(doc, dict):
            problems.append("exit 0 but stdout is not a JSON object")
        elif S is None:
            problems.append("oracle could not parse the archive but validator succeeded")
        else:
            compared = True
            limits, deriv = expected(S, ov)
            got_sb, ok1 = _find(doc, "storedBytes")
            got_l, ok2 = _find(doc, "limits")
            got_d, ok3 = _find(doc, "limitDerivation")
            if not ok1 or got_sb != S:
                problems.append("storedBytes: got %r want %r" % (got_sb, S))
            if not ok2 or got_l != limits:
                diff = {k: [(got_l or {}).get(k), limits.get(k)] for k in limits
                        if not isinstance(got_l, dict) or got_l.get(k) != limits[k]}
                if isinstance(got_l, dict):
                    diff.update({k: [got_l[k], None] for k in got_l if k not in limits})
                problems.append("limits differ: %s" % json.dumps(diff, sort_keys=True))
            if not ok3 or got_d != deriv:
                problems.append("limitDerivation differs")
    else:
        err = doc.get("error") if isinstance(doc, dict) else None
        if not isinstance(err, dict):
            err = doc if isinstance(doc, dict) else {}
        code = err.get("code") or err.get("category") or err.get("kind")
        row["errorCode"] = code
        if code == "resource_limit":
            lim = err.get("limit")
            if isinstance(lim, dict) and lim.get("limit") in OVERRIDABLE:
                key = lim["limit"]
                compared = True
                # An overridden key's value and derivation do not depend on the archive size.
                if S is None and key not in ov:
                    problems.append("oracle could not parse the archive")
                else:
                    limits, deriv = expected(S or 0, ov)
                    want_v = limits[key]
                    want_d = derivation_label(key, deriv, ov)
                    if want_v is None:
                        problems.append("%s refused but the oracle has no cap set" % key)
                    if lim.get("value") != want_v:
                        problems.append("error.limit.value %r want %r" % (lim.get("value"), want_v))
                    if lim.get("derivation") != want_d:
                        problems.append("error.limit.derivation %r want %r" % (
                            lim.get("derivation"), want_d))
                    suffix = " (limit %s=%s, %s)" % (key, want_v, want_d)
                    msg = err.get("message")
                    if not isinstance(msg, str) or not msg.endswith(suffix):
                        problems.append("message does not end with %r: %r" % (suffix, msg))
    row["disagreements"] = problems
    return row, compared, bool(problems)


def cmd_check(args):
    overrides = []
    seen = set()
    for item in args.maxVolume or []:
        k, _, v = item.partition("=")
        if k not in OVERRIDABLE or not v.isdigit() or int(v) > U64_MAX or k in seen:
            print("bad --maxVolume %r" % item, file=sys.stderr)
            return 2
        seen.add(k)
        overrides.append((k, int(v)))
    archives = list(args.archives)
    if args.list:
        with open(args.list, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        for item in data:
            if isinstance(item, str):
                archives.append(item)
            elif isinstance(item, dict) and "archive" in item:
                archives.append(item["archive"])
    n_cmp = n_bad = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futs = [pool.submit(check_one, args.binary, a, overrides, args.timeout) for a in archives]
        for fut in futs:
            row, compared, bad = fut.result()
            n_cmp += 1 if compared else 0
            n_bad += 1 if bad else 0
            print(json.dumps(row, sort_keys=True), flush=True)
    print(json.dumps({"archives": len(archives), "compared": n_cmp, "disagreements": n_bad}),
          flush=True)
    return 1 if n_bad else 0


# --------------------------------------------------------------------------- self-test

def _stored_zip_with_cd(entries, force_z64_eocd=False):
    """Hand-build a ZIP whose central directory declares ZIP64 sizes.

    entries: list of (name, size) ; no member data is written for sizes >= 4 GiB-1
    (parse never reads data), local headers are minimal.
    """
    body = io.BytesIO()
    cd = io.BytesIO()
    for name, size in entries:
        nb = name.encode()
        off = body.tell()
        body.write(struct.pack("<IHHHHHIIIHH", 0x04034B50, 45, 0, 0, 0, 0x21, 0, 0, 0, len(nb), 0))
        body.write(nb)
        if size >= 0xFFFFFFFF:
            extra = struct.pack("<HHQQ", 1, 16, size, size)
            csz = usz = 0xFFFFFFFF
        else:
            extra = b""
            csz = usz = size
        cd.write(struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50, 45, 45, 0, 0, 0, 0x21, 0, csz,
                             usz, len(nb), len(extra), 0, 0, 0, 0, off))
        cd.write(nb)
        cd.write(extra)
    cd_off = body.tell()
    cd_bytes = cd.getvalue()
    n = len(entries)
    out = body.getvalue() + cd_bytes
    z64 = force_z64_eocd or n >= 0xFFFF
    if z64:
        z64_off = len(out)
        out += struct.pack("<IQHHIIQQQQ", 0x06064B50, 44, 45, 45, 0, 0, n, n, len(cd_bytes), cd_off)
        out += struct.pack("<IIQI", 0x07064B50, 0, z64_off, 1)
        out += struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, 0xFFFF, 0xFFFF, 0xFFFFFFFF,
                           0xFFFFFFFF, 7) + b"comment"
    else:
        out += struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, n, n, len(cd_bytes), cd_off, 0)
    return out


def cmd_self_test(_args):
    import zipfile
    fails = []

    def ok(cond, msg):
        if not cond:
            fails.append(msg)

    # Class A / B defaults and shape
    lim, der = expected(0, {})
    ok(len(lim) == 21, "21 keys")
    ok(all(lim[k] is None for k in CLASS_B), "class B null by default")
    ok(der["model"] == 1 and len(der["budgets"]) == 7, "derivation shape")
    ok(lim["liveDocumentBytes"] == 268435456, "liveDocumentBytes")
    ok(json.dumps(der, sort_keys=True) == json.dumps(expected(5, {})[1], sort_keys=True),
       "derivation independent of S below crossovers")

    # crossover table, and that the table equals the first S with S*p//q > floor
    for k, (floor, p, q) in CLASS_C.items():
        c = CROSSOVER[k]
        first = None
        # smallest S with S*p//q > floor  <=>  S*p >= (floor+1)*q
        first = -(-((floor + 1) * q) // p)
        ok(first == c, "%s crossover table %d vs computed %d" % (k, c, first))
        for S, want_src, want_val in ((c - 1, "floor", floor), (c, "scaled", c * p // q),
                                      (c + 1, "scaled", (c + 1) * p // q)):
            lim, der = expected(S, {})
            b = der["budgets"][k]
            ok(lim[k] == want_val and b["source"] == want_src,
               "%s S=%d got %r/%s want %r/%s" % (k, S, lim[k], b["source"], want_val, want_src))
            ok(b["floor"] == floor and b["perStoredByte"] == [p, q], "%s rule fields" % k)
        # tie: S where S*p//q == floor exactly -> floor
        lim, der = expected(c - 1, {})
        ok(lim[k] == floor and der["budgets"][k]["source"] == "floor", "%s tie" % k)
        # saturation
        lim, der = expected(U64_MAX, {})
        want = min(U64_MAX * p // q, U64_MAX)
        ok(lim[k] == want, "%s saturation" % k)
        if p > q:
            ok(lim[k] == U64_MAX and der["budgets"][k]["source"] == "scaled", "%s saturated" % k)
        # explicit below floor, zero, above
        for v in (0, 1, floor - 1, floor, U64_MAX):
            lim, der = expected(10 ** 12, {k: v})
            ok(lim[k] == v and der["budgets"][k]["source"] == "explicit", "%s explicit %d" % (k, v))
            ok(der["budgets"][k]["floor"] == floor and der["budgets"][k]["perStoredByte"] == [p, q],
               "%s explicit keeps rule" % k)
            for other in CLASS_C:
                if other != k:
                    ok(der["budgets"][other]["source"] != "explicit", "only %s explicit" % k)
    for k in CLASS_B:
        lim, der = expected(100, {k: 0})
        ok(lim[k] == 0, "class B explicit 0")
        lim, der = expected(100, {k: U64_MAX})
        ok(lim[k] == U64_MAX, "class B max")
    for bad in ({"jsonBytes": 1}, {"nope": 1}, {"references": -1}, {"references": 2 ** 64},
                {"references": True}):
        try:
            expected(1, bad)
            fails.append("accepted bad override %r" % (bad,))
        except ValueError:
            pass
    ok(derivation_label("archiveEntries", expected(1, {})[1], {}) == "cap", "cap label")

    # parse: zipfile STORED
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_STORED) as zf:
        zf.writestr("a.bin", b"x" * 100)
        zf.writestr("b.bin", b"")
        zf.writestr("c/d.bin", b"y" * 1234)
    ok(parse(buf.getvalue()) == (3, 1334), "parse zipfile stored: %r" % (parse(buf.getvalue()),))
    ok(parse(io.BytesIO(buf.getvalue())) == (3, 1334), "parse file object")
    # parse with archive comment
    buf2 = io.BytesIO()
    with zipfile.ZipFile(buf2, "w", zipfile.ZIP_STORED) as zf:
        zf.comment = b"PK\x05\x06 not an eocd"
        zf.writestr("a", b"abc")
    ok(parse(buf2.getvalue()) == (1, 3), "parse with comment")
    # zipfile forced zip64 (ZIP64 extra in central + local)
    buf3 = io.BytesIO()
    with zipfile.ZipFile(buf3, "w", zipfile.ZIP_STORED) as zf:
        with zf.open("z.bin", "w", force_zip64=True) as w:
            w.write(b"q" * 77)
        zf.writestr("plain", b"12345")
    ok(parse(buf3.getvalue()) == (2, 82), "parse forced zip64: %r" % (parse(buf3.getvalue()),))
    # hand-built ZIP64 central directory
    big = 5 * 2 ** 30 + 123
    raw = _stored_zip_with_cd([("big.bin", big), ("small", 10), ("exact", 0xFFFFFFFF)],
                              force_z64_eocd=True)
    ok(parse(raw) == (3, big + 10 + 0xFFFFFFFF), "parse hand zip64: %r" % (parse(raw),))
    raw = _stored_zip_with_cd([("big.bin", big), ("small", 10)], force_z64_eocd=False)
    ok(parse(raw) == (2, big + 10), "parse hand zip64 extra with classic EOCD")
    # empty archive
    ok(parse(b"PK\x05\x06" + b"\0" * 18) == (0, 0), "empty zip")
    # >= 0xFFFF entries via ZIP64 EOCD
    raw = _stored_zip_with_cd([("e%d" % i, 1) for i in range(0xFFFF + 3)])
    ok(parse(raw) == (0xFFFF + 3, 0xFFFF + 3), "65538 entries")

    print(json.dumps({"selfTest": "fail" if fails else "ok", "failures": fails}))
    return 1 if fails else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("--binary", required=True)
    c.add_argument("--maxVolume", action="append", metavar="KEY=N")
    c.add_argument("--list")
    c.add_argument("--jobs", type=int, default=2)
    c.add_argument("--timeout", type=float, default=3600.0)
    c.add_argument("archives", nargs="*")
    c.set_defaults(fn=cmd_check)
    t = sub.add_parser("self-test")
    t.set_defaults(fn=cmd_self_test)
    p = sub.add_parser("parse")
    p.add_argument("archive")
    p.set_defaults(fn=lambda a: (print(json.dumps(dict(zip(("entries", "storedBytes"), parse(a.archive))))), 0)[1])
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
