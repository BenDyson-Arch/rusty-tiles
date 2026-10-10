#!/usr/bin/env python3
"""Independent authored controls of the F1d2 exact complete-cover reader."""
from fractions import Fraction as F
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[4]
CHECKER = ROOT / 'tests/f1d2_certificate.py'


def main():
    pinned = hashlib.sha256(CHECKER.read_bytes()).hexdigest()
    spec = importlib.util.spec_from_file_location('review_complete_cover', CHECKER)
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    whole = ((0, 0, 0), (8, 0, 0), (0, 8, 0))
    quarters = [((0, 0, 0), (4, 0, 0), (0, 4, 0)),
                ((4, 0, 0), (8, 0, 0), (4, 4, 0)),
                ((0, 4, 0), (4, 4, 0), (0, 8, 0)),
                ((4, 0, 0), (4, 4, 0), (0, 4, 0))]
    zero = checker.certify_regions([([whole], quarters)], F(0))
    assert (zero['patch_face_tests'], zero['accepted_patches'], zero['max_depth']) == (24, 8, 1)
    assert F(zero['ideal_cover_squared']) == 0
    shifted = [tuple((x, y, F(1, 8)) for x, y, z in face) for face in quarters]
    offset = checker.certify_regions([([whole], shifted)], F(1, 8))
    assert F(offset['ideal_cover_squared']) == F(1, 64)
    ring = [(whole[0], whole[1], whole[0]),
            (whole[1], whole[2], whole[1]),
            (whole[2], whole[0], whole[2])]
    # Every original corner belongs to the target union, but its interior does not.
    assert all(min(checker.point_triangle_squared(p, face) for face in ring) == 0 for p in whole)
    centroid = (F(8, 3), F(8, 3), F(0))
    gap = min(checker.point_triangle_squared(centroid, face) for face in ring)
    assert gap == F(32, 9)
    refused = []
    for label, regions, scalar, kwargs in [
        ('positive-offset-false-zero', [([whole], shifted)], F(0), {'max_depth': 5}),
        ('interior-hole-false-zero', [([whole], ring)], F(0), {'max_depth': 5}),
        ('cover-needs-depth-one', [([whole], quarters)], F(0), {'max_depth': 0}),
        ('cover-needs-work-24', [([whole], quarters)], F(0), {'max_tests': 23})]:
        try: checker.certify_regions(regions, scalar, **kwargs)
        except checker.CertificateUnproven as error:
            refused.append({'name': label, 'classification': 'unproven', 'message': str(error)})
        else: raise AssertionError('insensitive complete-cover reader: ' + label)
    actual = {'error_metres': 0, 'patch_face_tests': 24, 'accepted_patches': 8, 'max_depth': 1}
    checker.validate_metrics(actual, zero)
    rejected_metrics = []
    for key, value in [('patch_face_tests', 23), ('accepted_patches', 5),
                       ('max_depth', 0), ('patch_face_tests', True), ('max_depth', 25)]:
        altered = dict(actual); altered[key] = value
        try: checker.validate_metrics(altered, zero)
        except checker.CertificateError: rejected_metrics.append({'key': key, 'value': value})
        else: raise AssertionError('insensitive metric guard')
    # Necessary inequalities cannot authenticate the producer's exact counters.
    inflated = dict(actual, patch_face_tests=80, accepted_patches=11, max_depth=2)
    checker.validate_metrics(inflated, zero)
    assert hashlib.sha256(CHECKER.read_bytes()).hexdigest() == pinned
    print(json.dumps({'scope': 'Authored exact supports and checker controls only; no converter artifact acceptance or Rust execution.',
                     'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                     'checker_sha256': pinned, 'equal_support': zero, 'offset_support': offset,
                     'interior_gap_squared': str(gap), 'bounded_refusals': refused,
                     'rejected_metric_controls': rejected_metrics,
                     'inflated_metrics_can_pass_necessary_inequalities': inflated,
                     'metrics_limit': 'Independent proof metrics are necessary lower bounds, not authentication of exact producer counters.',
                     'passed': True}, indent=2))


if __name__ == '__main__': main()
