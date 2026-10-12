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
  PrecheckPassed                only with --precheck-only: the precheck passed and
                                streaming was not run; never a verdict
`c1Caveats` lists conditions under which production may stop in C1 first.

With --implicit (C2 slice 3, c2o/implicit.py) the archive first goes through
O-implicit (implicit_read/oracle/i1_oracle.py) at the #160 defaults, or at the
--i1-limit overrides (an I1 control's manifest limits). A non-Accepted I1
verdict is the outcome (InvalidInput, Unsupported or ResourceLimit with
"phase": "I1"). Otherwise every emitted implicit content is a reference, and
the default containmentWorkUnits is C2-WB's frozen 2^29.
Exit status: 0 when a report was written, 2 on usage errors.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from c2o.check import DEFAULT_LIMITS, ORACLE_LIMITS, Oracle  # noqa: E402
from c2o.implicit import I1_DEFAULTS_160, ImplicitOracle, i1_module  # noqa: E402


def _limits(items, allowed, what):
    out = {}
    for item in items or []:
        name, sep, value = item.partition("=")
        if not sep or name not in allowed:
            raise SystemExit("unknown %s %r; allowed: %s" % (what, name, ", ".join(sorted(allowed))))
        out[name] = int(value)
    return out


def _i1_names():
    return set(i1_module().DEFAULT_LIMITS) | set(I1_DEFAULTS_160)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("archive")
    parser.add_argument("--json", action="store_true", help="print the JSON report (default)")
    parser.add_argument("--limit", action="append", metavar="NAME=VALUE",
                        help="ValidationLimits C2 cap, e.g. containmentSpanBits=29")
    parser.add_argument("--oracle-limit", action="append", metavar="NAME=VALUE",
                        help="the oracle's own work bounds, e.g. maxVertexLevelTests=1000")
    parser.add_argument("--precheck-only", action="store_true",
                        help="stop after a passed precheck (outcome PrecheckPassed: units only, no verdict)")
    parser.add_argument("--no-detail", action="store_true",
                        help="omit the per-reference precheck rows (totals and terms are kept)")
    parser.add_argument("--implicit", action="store_true",
                        help="C2 slice 3: expand implicit roots through O-implicit (c2o/implicit.py)")
    parser.add_argument("--i1-limit", action="append", metavar="NAME=VALUE",
                        help="with --implicit: an O-implicit limit, e.g. hierarchy_depth=12")
    args = parser.parse_args(argv)
    try:
        limits = _limits(args.limit, DEFAULT_LIMITS, "limit")
        oracle_limits = _limits(args.oracle_limit, ORACLE_LIMITS, "oracle limit")
        i1_limits = _limits(args.i1_limit, _i1_names(), "I1 limit")
        if i1_limits and not args.implicit:
            raise SystemExit("--i1-limit needs --implicit")
    except SystemExit as err:
        print(err, file=sys.stderr)
        return 2
    if args.implicit:
        report = ImplicitOracle(args.archive, limits, oracle_limits, i1_limits, precheck_only=args.precheck_only,
                                detail=not args.no_detail).run()
    else:
        report = Oracle(args.archive, limits, oracle_limits, precheck_only=args.precheck_only,
                        detail=not args.no_detail).run()
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
