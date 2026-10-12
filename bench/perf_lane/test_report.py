"""Unit tests for report.py's extraction, delta, flag and rendering logic.

Run: python3 -I bench/perf_lane/test_report.py
"""
import pathlib, sys, unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import report  # noqa: E402

GOOD = dict(wall=1.0, rss=100.0, bytes=1000, vwall=0.2, load=True, errors=0, tiles=10, secs=2.0)


def metrics(**changes):
    return dict(GOOD, **changes)


def process_side(ok=True, wall=1.0, rss=100.0, out=1000, skipped=None):
    if skipped: return dict(skipped=skipped)
    side = dict(ok=ok, wallSeconds=wall, peakRssMiB=rss, samples=[dict(exitCode=0 if ok else 2)])
    if ok: side['outputBytes'] = out
    else: side['log'] = 'reading input\nerror: unexpected argument --jobs\n'
    return side


def meta(label=None, release=True):
    return dict(date='2026-10-12', label=label, corpus='c' * 64, head=dict(commit='d1874ef' + '0' * 33, sha256='a' * 64),
                release=dict(tag='v0.3.0', commit='99b9605' + '0' * 33, sha256='b' * 64, build='cargo build') if release else None,
                environment=dict(cpu='Test CPU', cpuAffinity=[0, 1], niceness=10, repeat=3, loadAverageBefore=[1.0, 1.0, 1.0]),
                cesium=dict(cesium='1.146.0', playwright='1.63.0', chromium='chrome'), bundle=dict(path='~/raw.tar.zst', sha256='e' * 64))


class Extraction(unittest.TestCase):
    def test_side_metrics_reads_process_validate_and_cesium(self):
        process = dict(cases=[
            dict(case='vector', sides=dict(base=process_side(), head=process_side(wall=2.0))),
            dict(case='validate:vector', sides=dict(base=process_side(wall=0.1), head=process_side(wall=0.3))),
            dict(case='mesh-local', sides=dict(base=process_side(skipped='no mesh-local-to-3tz command'), head=process_side())),
            dict(case='raster', sides=dict(base=process_side(ok=False), head=process_side())),
        ])
        cesium = dict(cases=[dict(name='vector', loaded=True, secondsToLoaded=2.5, console=dict(errors=['boom'], warnings=[]),
                                  problems=['console errors: 1'],
                                  requests=dict(content=dict(requests=4), subtree=dict(requests=1), imagery=dict(requests=0)))])
        head = report.side_metrics(process, cesium, 'head')
        self.assertEqual(head['vector'], dict(wall=2.0, rss=100.0, bytes=1000, vwall=0.3, load=True, errors=1, tiles=5,
                                              secs=2.5, problems=['console errors: 1']))
        base = report.side_metrics(process, None, 'base')
        self.assertEqual(base['mesh-local'], dict(na='no mesh-local-to-3tz command'))
        self.assertEqual(base['raster'], dict(failed='exit 2: error: unexpected argument --jobs'))
        self.assertNotIn('load', base['vector'])


class Deltas(unittest.TestCase):
    def test_pct(self):
        self.assertEqual(report.pct(2.0, 2.5), '+25.0%')
        self.assertEqual(report.pct(4.0, 3.0), '-25.0%')
        self.assertEqual(report.pct(0, 0), '')
        self.assertEqual(report.pct(0, 5), 'new')
        self.assertEqual(report.pct(None, 1), '')

    def test_delta_cells(self):
        self.assertEqual(report.delta('wall', GOOD, metrics(wall=1.5), bold=True), '**+50.0%**')
        self.assertEqual(report.delta('load', GOOD, metrics(errors=2), bold=False), '+2 errors')
        self.assertEqual(report.delta('load', GOOD, metrics(load=False, errors=3), bold=False), 'changed')
        self.assertEqual(report.delta('load', GOOD, GOOD, bold=False), '')
        self.assertEqual(report.delta('wall', dict(na='x'), GOOD, bold=False), '')

    def test_cells(self):
        self.assertEqual(report.cell('bytes', GOOD), '1,000')
        self.assertEqual(report.cell('load', metrics(load=False, errors=3)), 'FAIL (3)')
        self.assertEqual(report.cell('load', GOOD), 'ok (0)')
        self.assertEqual(report.cell('load', dict(wall=1.0)), '—')
        self.assertEqual(report.chromium('/x/chromium-1243/chrome-linux64/chrome'), '1243')
        self.assertEqual(report.cell('wall', dict(na='no command')), 'n/a')
        self.assertEqual(report.cell('wall', dict(failed='exit 2')), 'failed')
        self.assertEqual(report.cell('secs', dict(wall=1.0)), '—')
        self.assertEqual(report.cell('wall', None), '—')


class Flags(unittest.TestCase):
    def flags(self, ref, head, explanations=()):
        return report.compare('vector', 'milestone', ref, head, list(explanations))

    def test_ratio_and_floor(self):
        self.assertEqual(self.flags(GOOD, metrics(wall=1.25)), [])  # exactly 1.25x is not a flag
        self.assertEqual([f['metric'] for f in self.flags(GOOD, metrics(wall=1.3))], ['wall'])
        self.assertEqual(self.flags(metrics(wall=0.1), metrics(wall=0.19)), [])  # +90% but under the 0.1 s floor
        self.assertEqual([f['metric'] for f in self.flags(GOOD, metrics(rss=126.0))], ['rss'])  # +26%, +26 MiB
        self.assertEqual(self.flags(metrics(rss=40.0), metrics(rss=55.0)), [])  # +37.5% but under 16 MiB
        self.assertEqual([f['metric'] for f in self.flags(GOOD, metrics(bytes=1251))], ['bytes'])  # 0-byte floor

    def test_validate_and_advisory_keys(self):
        found = {f['metric']: f for f in self.flags(GOOD, metrics(vwall=0.5, secs=4.0, tiles=20))}
        self.assertEqual(found['vwall']['key'], 'validate:vector.wallSeconds')
        self.assertTrue(found['vwall']['gated'])
        self.assertEqual(found['secs']['key'], 'vector.secondsToLoaded')
        self.assertFalse(found['secs']['gated'])
        self.assertFalse(found['tiles']['gated'])

    def test_explain_uses_fnmatch(self):
        [flag] = self.flags(GOOD, metrics(wall=2.0), [('vector*.wallSeconds', 'expected')])
        self.assertEqual(flag['explanation'], 'expected')
        [flag] = self.flags(GOOD, metrics(wall=2.0), [('raster.*', 'other')])
        self.assertIsNone(flag['explanation'])

    def test_head_skip_or_failure(self):
        [flag] = self.flags(GOOD, dict(na='no command'))
        self.assertEqual((flag['metric'], flag['key'], flag['text']), ('exit', 'vector.exit', 'head no command'))
        self.assertEqual(self.flags(dict(na='no command'), GOOD), [])  # nothing to compare against
        self.assertEqual(self.flags(None, GOOD), [])

    def test_gate_reference_and_cesium(self):
        columns = dict(release=dict(vector=metrics(wall=0.5)), milestone=dict(vector=GOOD),
                       head=dict(vector=metrics(wall=1.1, problems=['console errors: 1'])))
        flags, failures = report.all_flags(columns, [])
        self.assertEqual(sorted((f['ref'], f['fails']) for f in flags), [('cesium', True), ('release', False)])
        self.assertEqual([f['ref'] for f in failures], ['cesium'])
        # Without a milestone, the release column gates.
        columns['milestone'] = None
        _, failures = report.all_flags(columns, [])
        self.assertEqual(sorted(f['ref'] for f in failures), ['cesium', 'release'])
        # An explanation clears a gated metric flag, but never a Cesium failure.
        _, failures = report.all_flags(columns, [('vector.*', 'reason')])
        self.assertEqual([f['ref'] for f in failures], ['cesium'])


class Rendering(unittest.TestCase):
    def columns(self):
        return dict(release=dict(vector=metrics(wall=0.5), **{'mesh-local': dict(na='no mesh-local-to-3tz command')}),
                    milestone=dict(vector=GOOD, **{'mesh-local': GOOD}), head=dict(vector=metrics(wall=1.1), **{'mesh-local': GOOD}))

    def test_round_trip_and_milestones(self):
        first, failures = report.render(meta('first'), self.columns(), (None, {}, []), [])
        self.assertEqual(failures, [])
        info, cases, rows = report.parse_previous(first)
        self.assertEqual(info['label'], 'first')
        self.assertEqual(cases['vector']['wall'], 1.1)
        self.assertEqual(cases['mesh-local'], {k: v for k, v in GOOD.items()})
        self.assertEqual(len(rows), 1)
        self.assertIn('| 2026-10-12 | `d1874ef` | first | 2.10 |', rows[0])
        # A non-milestone run keeps the stored milestone and its rows unchanged.
        columns = self.columns(); columns['milestone'] = cases
        second, _ = report.render(meta(), columns, (info, cases, rows), [])
        self.assertEqual(report.parse_previous(second), (info, cases, rows))
        # A later milestone appends a row; rerunning the same commit replaces it.
        later = meta('second'); later['head']['commit'] = 'f00ba12' + '0' * 33
        third, _ = report.render(later, columns, (info, cases, rows), [])
        self.assertEqual(len(report.parse_previous(third)[2]), 2)
        again, _ = report.render(meta('first again'), columns, (info, cases, rows), [])
        self.assertEqual([r.split('|')[3].strip() for r in report.parse_previous(again)[2]], ['first again'])

    def test_table_cells_and_notes(self):
        text, _ = report.render(meta(), self.columns(), (None, {}, []), [])
        self.assertIn('| vector | v0.3.0 | last milestone | head | Δ v0.3.0 | Δ milestone |', text)
        self.assertIn('| wall s | 0.500 | 1.000 | 1.100 | **+120.0%** | +10.0% |', text)
        self.assertIn('| wall s | n/a | 1.000 | 1.000 |  | +0.0% |', text)
        self.assertIn('- mesh-local on v0.3.0: no mesh-local-to-3tz command', text)
        self.assertIn('- vector vs v0.3.0: wall s 0.500 -> 1.100 (+120.0%) — not gated', text)
        self.assertNotIn(report.BLOCK_START, text)  # no milestone yet and none recorded

    def test_regression_is_listed_first(self):
        columns = self.columns(); columns['head']['vector'] = metrics(wall=2.0)
        text, failures = report.render(meta(), columns, (None, {}, []), [])
        self.assertEqual(len(failures), 1)
        flags = text.split('## Flags\n\n')[1].split('\n\n')[0].splitlines()
        self.assertTrue(flags[0].endswith('— REGRESSION'), flags)


if __name__ == '__main__':
    unittest.main()
