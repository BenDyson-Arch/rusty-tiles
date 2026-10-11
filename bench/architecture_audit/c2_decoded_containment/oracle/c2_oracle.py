#!/usr/bin/env python3
"""C2 O-explicit oracle CLI.

    python3 -I oracle/c2_oracle.py ARCHIVE.3tz --json [--limit name=value ...]

Prints one JSON report. `outcome` is one of:
  Contained                     every drawn vertex is inside every tested volume
  Outside        (facts)        first proven escape in streaming order
  NotInspected   (reasons)      exclusion reasons, or the precheck reason
  InvalidNode    (facts)        malformed consumed scene/node record (InvalidInput)
  InvalidInput   (facts)        tileset defect that is InvalidInput (v3.3 (8)): a
                                non-affine/non-finite tile transform or an
                                overflowing number literal
  Undecided      (why)          the oracle could not decide (its own limits, an
                                unmodelled format, or data C1 should reject);
                                never a verdict
`c1Caveats` lists conditions under which production may stop in C1 first.
Exit status: 0 when a report was written, 2 on usage errors.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from c2o.check import DEFAULT_LIMITS, ORACLE_LIMITS, Oracle  # noqa: E402


def _limits(items, allowed, what):
    out = {}
    for item in items or []:
        name, sep, value = item.partition("=")
        if not sep or name not in allowed:
            raise SystemExit("unknown %s %r; allowed: %s" % (what, name, ", ".join(sorted(allowed))))
        out[name] = int(value)
    return out


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("archive")
    parser.add_argument("--json", action="store_true", help="print the JSON report (default)")
    parser.add_argument("--limit", action="append", metavar="NAME=VALUE",
                        help="ValidationLimits C2 cap, e.g. containmentSpanBits=29")
    parser.add_argument("--oracle-limit", action="append", metavar="NAME=VALUE",
                        help="the oracle's own work bounds, e.g. maxVertexLevelTests=1000")
    args = parser.parse_args(argv)
    try:
        limits = _limits(args.limit, DEFAULT_LIMITS, "limit")
        oracle_limits = _limits(args.oracle_limit, ORACLE_LIMITS, "oracle limit")
    except SystemExit as err:
        print(err, file=sys.stderr)
        return 2
    report = Oracle(args.archive, limits, oracle_limits).run()
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
