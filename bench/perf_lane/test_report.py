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
    side = dict(ok=ok, wallSeconds=wall, peakRssMiB=rss, samples=[dict(exitCode=0 if ok else 101)])
    if ok: side['outputBytes'] = out
    else: side['log'] = "reading input\nthread 'main' panicked at src/raster.rs:1:1\n"
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
            dict(case='terrain', sides=dict(base=dict(ok=False, samples=[dict(exitCode=2)], log='error: the following required '
                 'arguments were not provided:\n  --maxZoom <MAX_ZOOM>\n\nUsage: rusty-tiles terrain\n'), head=process_side())),
        ])
        cesium = dict(cases=[dict(name='vector', loaded=True, secondsToLoaded=2.5, console=dict(errors=['boom'], warnings=[]),
                                  problems=['console errors: 1'],
                                  requests=dict(content=dict(requests=4), subtree=dict(requests=1), imagery=dict(requests=0)))])
        head = report.side_metrics(process, cesium, 'head')
        self.assertEqual(head['vector'], dict(wall=2.0, rss=100.0, bytes=1000, vwall=0.3, load=True, errors=1, tiles=5,
                                              secs=2.5, problems=['console errors: 1']))
        base = report.side_metrics(process, None, 'base')
        self.assertEqual(base['mesh-local'], dict(na='no mesh-local-to-3tz command'))
        self.assertEqual(base['raster'], dict(failed="exit 101: thread 'main' panicked at src/raster.rs:1:1"))
        self.assertEqual(base['terrain'], dict(na='usage error: error: the following required arguments were not provided: '
                                                  '--maxZoom <MAX_ZOOM>'))
        self.assertNotIn('load', base['vector'])

    def test_contended(self):
        side = lambda wall, foreign: dict(ok=True, wallSeconds=wall, foreignCpuSecondsAtBestWall=foreign)
        process = dict(cases=[dict(case='vector', sides=dict(base=side(1.0, 0.2), head=side(1.0, 0.21))),
                              dict(case='mesh', sides=dict(base=dict(skipped='x'), head=side(0.1, 0.5)))])
        self.assertEqual(report.contended(process), ['mesh head', 'vector head'])


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

    def test_head_failure_is_flagged_without_references(self):
        failed = dict(failed='exit 101: panicked')
        for columns in (dict(release=None, milestone=None, head=dict(vector=failed)),
                        dict(release=dict(vector=dict(na='no command')), milestone={}, head=dict(vector=failed)),
                        dict(release=dict(vector=GOOD), milestone=dict(vector=GOOD), head=dict(vector=failed))):
            flags, failures = report.all_flags(columns, [])
            self.assertEqual([(f['ref'], f['key'], f['text']) for f in flags],
                             [('head', 'vector.exit', 'head failed: exit 101: panicked')])
            self.assertEqual(len(failures), 1)
        _, failures = report.all_flags(dict(release=None, milestone=None, head=dict(vector=failed)), [('vector.exit', 'known')])
        self.assertEqual(failures, [])

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


class Explanations(unittest.TestCase):
    def test_recorded_explanations_are_kept_and_new_ones_win(self):
        merged = report.merge_explanations(['validate:*.wallSeconds=C2 checks', 'raster.exit=no GDAL'],
                                           ['validate:*.wallSeconds=classified: payload checks'])
        self.assertEqual(merged, [['validate:*.wallSeconds', 'classified: payload checks'], ['raster.exit', 'no GDAL']])
        [flag] = report.compare('vector', 'milestone', GOOD, metrics(vwall=0.9), merged)
        self.assertEqual(flag['explanation'], 'classified: payload checks')


class CpuChoice(unittest.TestCase):
    PAIRS = [(4, 5), (6, 7), (12, 13), (14, 15)]

    def test_pick_pair(self):
        idle = {4: .95, 5: .97, 6: .99, 7: .99, 12: .5, 13: .99, 14: .91, 15: .2}
        self.assertEqual(report.pick_pair(idle, self.PAIRS, (4, 5), .9), (4, 5))  # current pair still qualifies
        self.assertEqual(report.pick_pair(idle, self.PAIRS, None, .9), (6, 7))  # else the idlest qualifying pair
        self.assertEqual(report.pick_pair(idle, self.PAIRS, (12, 13), .9), (6, 7))  # one busy core disqualifies
        self.assertIsNone(report.pick_pair({c: .5 for c in idle}, self.PAIRS, None, .9))

    def test_sibling_pairs_stay_inside_the_pool(self):
        for pair in report.sibling_pairs({4, 5, 6, 7, 12, 13, 14, 15}):
            self.assertEqual(len(pair), 2)
            self.assertTrue(set(pair) <= {4, 5, 6, 7, 12, 13, 14, 15})


class ParsePrevious(unittest.TestCase):
    def setUp(self):
        columns = dict(release=None, milestone=None, head=dict(vector=GOOD))
        self.text, _ = report.render(meta('first'), columns, (None, {}, []), [])
        self.expected = report.parse_previous(self.text)

    def edited(self, old, new):
        self.assertIn(old, self.text)
        return self.text.replace(old, new)

    def assertFails(self, text, message):
        with self.assertRaises(SystemExit) as raised: report.parse_previous(text)
        self.assertIn(message, str(raised.exception))

    def test_empty_report_starts_the_log(self):
        self.assertEqual(report.parse_previous(''), (None, {}, []))
        self.assertEqual(report.parse_previous('\n  \n'), (None, {}, []))

    def test_indented_and_trailing_whitespace(self):
        text = '\n'.join('  ' + line + '  ' for line in self.text.splitlines())
        self.assertEqual(report.parse_previous(text), self.expected)

    def test_marker_and_heading_matched_by_prefix(self):
        text = self.edited(report.BLOCK_START, '<!-- perf-lane milestone data (hand edited)')
        text = text.replace('## Milestones', '## Milestones (trend log)')
        self.assertEqual(report.parse_previous(text), self.expected)

    def test_row_without_spaces(self):
        row = self.expected[2][0]
        compact = '|' + '|'.join(cell.strip() for cell in row.strip('|').split('|')) + '|'
        self.assertTrue(compact.startswith('|2026-10-12|'))
        self.assertEqual(report.parse_previous(self.edited(row, compact)), self.expected)

    def test_note_lines_are_skipped(self):
        row = self.expected[2][0]
        text = self.edited(row, 'Note: the 2026-10-11 row was seeded from legacy results.\n' + row + '\n> reviewed')
        self.assertEqual(report.parse_previous(text), self.expected)

    def test_missing_data_block_fails(self):
        start = self.text.index(report.BLOCK_START)
        self.assertFails(self.text[:start], 'expected one milestone data block, found 0')

    def test_unclosed_or_garbled_block_fails(self):
        self.assertFails(self.text.rstrip().removesuffix('-->'), 'not closed')
        self.assertFails(self.edited('case vector {', 'case vector {oops'), 'bad JSON')
        self.assertFails(self.edited('case vector', 'cases vector'), 'unrecognised data line')
        milestone = next(line for line in self.text.splitlines() if line.startswith('milestone '))
        self.assertFails(self.edited(milestone + '\n', ''), 'lacks the milestone line')

    def test_missing_rows_fail(self):
        self.assertFails(self.edited(self.expected[2][0] + '\n', ''), "'## Milestones' has no rows")
        self.assertFails(self.edited('## Milestones', '## Trend'), "expected one '## Milestones' section, found 0")
        self.assertFails(self.edited('`d1874ef`', '`0000000`'), "no Milestones row for the data block's commit d1874ef")
        self.assertFails(self.edited('| 2026-10-12 |', '| 12 Oct |'), 'unrecognised Milestones row')


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
        recorded = meta()
        recorded['pinned'] = {'process': dict(cpus=[6, 7], idle={'6': 0.995, '7': 0.96})}
        text, _ = report.render(recorded, self.columns(), (None, {}, []), [])
        self.assertIn('- Per-core idle over 5 s before each step (need ≥90%): process CPUs 6,7 100%/96%.', text)
        self.assertIn('| vector | v0.3.0 | last milestone | head | Δ v0.3.0 | Δ milestone |', text)
        self.assertIn('| wall s | 0.500 | 1.000 | 1.100 | **+120.0%** | +10.0% |', text)
        self.assertIn('| wall s | n/a | 1.000 | 1.000 |  | +0.0% |', text)
        self.assertIn('- v0.3.0: no mesh-local-to-3tz command (mesh-local)', text)
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
