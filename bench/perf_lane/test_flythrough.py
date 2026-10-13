"""Unit tests for the flythrough lane: path expansion, recipe parsing, percentile/hitch/pop math, noise and
aggregation, and the shipped configuration.

Run: python3 -I bench/perf_lane/test_flythrough.py
"""
import json, math, pathlib, shutil, subprocess, sys, unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import flythrough as fly  # noqa: E402
import flythrough_report as rep  # noqa: E402


def path(*segments):
    return dict(segments=list(segments))


class Paths(unittest.TestCase):
    def test_motion_frames_end_on_target_and_start_after_waypoint(self):
        steps = fly.expand_path('p', path({'waypoint': 'a', 'pose': {'heading': 0, 'range': 2}},
                                          {'frames': 4, 'to': {'heading': 360}}, {'waypoint': 'b'}))
        self.assertEqual(len(steps), 6)
        self.assertEqual([s.get('waypoint') for s in steps], ['a', None, None, None, None, 'b'])
        self.assertEqual([s['pose'][0] for s in steps], [0, 90, 180, 270, 360, 360])  # no wrap: a full orbit
        self.assertEqual(steps[0]['pose'], [0, -45, 2, 0, 0, 0])  # unset keys take DEFAULT_POSE

    def test_range_is_geometric_and_smooth_ease_is_symmetric(self):
        steps = fly.expand_path('p', path({'waypoint': 'a', 'pose': {'range': 4}},
                                          {'frames': 2, 'to': {'range': 1, 'east': 1}, 'ease': 'smooth'}))
        self.assertAlmostEqual(steps[1]['pose'][2], 2.0)  # halfway in log space
        self.assertAlmostEqual(steps[1]['pose'][3], 0.5)  # smoothstep(0.5) = 0.5
        quarter = fly.expand_path('p', path({'waypoint': 'a', 'pose': {}}, {'frames': 4, 'to': {'east': 1}, 'ease': 'smooth'}))
        self.assertAlmostEqual(quarter[1]['pose'][3], 0.15625)  # 3t^2 - 2t^3 at 0.25

    def test_waypoint_pose_is_an_absolute_cut(self):
        steps = fly.expand_path('p', path({'waypoint': 'a', 'pose': {'heading': 10, 'range': 2, 'north': 0.4}},
                                          {'frames': 2, 'to': {'east': 1}},
                                          {'waypoint': 'b', 'pose': {'east': 0.5}}))
        # unset keys take DEFAULT_POSE, not the pose the camera arrived with
        self.assertEqual(steps[-1], dict(pose=[0, -45, 3, 0.5, 0, 0], waypoint='b'))
        self.assertEqual(steps[2]['pose'], [10, -45, 2, 1, 0.4, 0])  # a motion 'to' keeps the current pose

    def test_teleport_returns_to_the_same_overview(self):
        steps = {s['waypoint']: s['pose'] for s in fly.load_config()['expanded']['snap-teleport']}
        self.assertEqual(steps['overview-again'], steps['overview'])
        self.assertEqual(steps['centre-close'][3:], [0, 0, 0])  # not left at the sw-close offset

    def test_deterministic(self):
        config = fly.load_config()
        again = fly.load_config()
        self.assertEqual(json.dumps(config['expanded']), json.dumps(again['expanded']))

    def test_invalid_paths(self):
        bad = [
            path({'frames': 3, 'to': {}}),  # must start with a waypoint
            path({'waypoint': 'a'}),  # first waypoint needs a pose
            path({'waypoint': 'a', 'pose': {}}, {'waypoint': 'a'}),  # repeated name
            path({'waypoint': 'a', 'pose': {'range': 0}}),
            path({'waypoint': 'a', 'pose': {'tilt': 1}}),
            path({'waypoint': 'a', 'pose': {}}, {'frames': 0, 'to': {}}),
            path({'waypoint': 'a', 'pose': {}}, {'frames': 2.5, 'to': {}}),
            path({'waypoint': 'a', 'pose': {}}, {'frames': 2, 'to': {}, 'ease': 'bounce'}),
            path({'waypoint': 'a', 'pose': {'heading': math.nan}}),
            dict(segments=[]),
        ]
        for item in bad:
            with self.subTest(item=item), self.assertRaises(ValueError): fly.expand_path('p', item)

    def test_shipped_paths(self):
        config = fly.load_config()
        self.assertEqual(set(config['paths']), {'orbit-far', 'approach', 'fast-pan', 'close-crawl', 'snap-teleport'})
        motion = {n: sum('waypoint' not in s for s in steps) for n, steps in config['expanded'].items()}
        self.assertEqual(motion, {'orbit-far': 600, 'approach': 420, 'fast-pan': 90, 'close-crawl': 900, 'snap-teleport': 0})
        self.assertEqual(set(config['profiles']), {'native', 'cpu4x', 'lowcache'})

    def test_run_order_rotates_recipes(self):
        cells = [('native', 'p', r) for r in 'abc']
        order = fly.run_order(['a', 'b', 'c'], cells, 3)
        self.assertEqual([o[3] for o in order], ['a', 'b', 'c', 'b', 'c', 'a', 'c', 'a', 'b'])
        self.assertEqual([o[0] for o in order], [1, 1, 1, 2, 2, 2, 3, 3, 3])

    def test_untimed_runs_are_separate_and_first(self):
        cells = [('native', 'p', 'a'), ('native', 'p', 'b'), ('cpu4x', 'p', 'b')]
        order = fly.run_order(['a', 'b'], cells, 1, {('native', 'p', 'a'): dict(capture=True, trace=False)})
        self.assertEqual(order, [(0, 'native', 'p', 'a'), (1, 'native', 'p', 'a'), (1, 'native', 'p', 'b'), (1, 'cpu4x', 'p', 'b')])


def names(every, kept=None):
    return dict(all=every, kept=kept or every)


class Matrix(unittest.TestCase):
    def shipped(self, preset='validate', recipes=None, paths=None, profiles=None):
        config = fly.load_config()
        loaded = [r['name'] for s in fly.load_recipe_sets(sorted((HERE / 'recipes').glob('*.json'))) for r in s['recipes']]
        return fly.expand_matrix(config['matrices'][preset], preset, names(loaded, recipes), names(list(config['paths']), paths),
                                 names(list(config['profiles']), profiles))

    def test_validate_preset_is_trimmed(self):
        recipes, cells, diagnostics, repeats = self.shipped()
        self.assertEqual(recipes, ['terrain-lod-sse0', 'terrain-skiplod-sse0', 'vector-default', 'vector-leaf16'])
        self.assertEqual(repeats, 3)
        self.assertEqual(sum(c[0] == 'native' for c in cells), 20)  # 4 recipes x 5 paths
        constrained = sorted(c for c in cells if c[0] != 'native')
        self.assertEqual(constrained, [(p, path, 'terrain-skiplod-sse0') for p in ('cpu4x', 'lowcache')
                                       for path in ('approach', 'snap-teleport')])
        self.assertEqual(sum(d['capture'] for d in diagnostics.values()), 20)
        traced = sorted(c for c, d in diagnostics.items() if d['trace'])
        self.assertEqual(traced, [('native', 'approach', r) for r in sorted(recipes)])  # one traced run per recipe
        self.assertEqual(len(fly.run_order(recipes, cells, repeats, diagnostics)), 20 + 24 * 3)

    def test_full_preset_covers_everything(self):
        recipes, cells, _, _ = self.shipped('full')
        self.assertEqual(len(cells), len(recipes) * 5 * 3)

    def test_filters_restrict_presets(self):
        recipes, cells, diagnostics, _ = self.shipped(profiles=['native'], paths=['approach'])
        self.assertEqual(cells, [('native', 'approach', r) for r in recipes])
        self.assertTrue(all(d['trace'] for d in diagnostics.values()))

    def test_invalid_matrices(self):
        available = (names(['a', 'b']), names(['p']), names(['native']))
        bad = [
            dict(cells=[]),
            dict(cells=[{'profiles': ['nope']}]),
            dict(cells=[{'profiles': '*', 'paths': '*', 'recipes': ['c']}]),
            dict(recipes=['c'], cells=[{}]),
            dict(cells=[{'views': '*'}]),
            dict(cells=[{}], repeats=0),
            dict(cells=[{}], extra=1),
        ]
        for item in bad:
            with self.subTest(item=item), self.assertRaises(ValueError): fly.expand_matrix(item, 'm', *available)
        _, cells, _, _ = fly.expand_matrix(dict(recipes=['b'], cells=[{}]), 'm', *available)
        self.assertEqual(cells, [('native', 'p', 'b')])


class Recipes(unittest.TestCase):
    def doc(self, *recipes):
        return dict(set='s', dataset='d', recipes=list(recipes))

    def test_like_copies_conversion_and_merges_viewer(self):
        parsed = fly.parse_recipes(self.doc(
            {'name': 'a', 'convert': ['terrain', '-o', '{out}'], 'viewer': {'tilesetOptions': {'x': 1}, 'maximumScreenSpaceError': 8}},
            {'name': 'b', 'like': 'a', 'viewer': {'tilesetOptions': {'skipLevelOfDetail': True}}}))
        a, b = parsed['recipes']
        self.assertEqual(b['convert'], a['convert'])
        self.assertEqual(b['viewer'], {'tilesetOptions': {'x': 1, 'skipLevelOfDetail': True}, 'maximumScreenSpaceError': 8})
        self.assertEqual(a['viewer']['tilesetOptions'], {'x': 1})  # the base is not mutated
        self.assertEqual(fly.conversion_key(a['convert']), fly.conversion_key(b['convert']))  # converted once

    def test_like_merges_scheduler(self):
        parsed = fly.parse_recipes(self.doc(
            {'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'scheduler': {'maximumRequests': 10}}},
            {'name': 'b', 'like': 'a', 'viewer': {'scheduler': {'throttleRequests': False}}}))
        self.assertEqual(parsed['recipes'][1]['viewer']['scheduler'], {'maximumRequests': 10, 'throttleRequests': False})
        self.assertEqual(parsed['recipes'][0]['viewer']['scheduler'], {'maximumRequests': 10})

    def test_output_instead_of_convert(self):
        parsed = fly.parse_recipes(self.doc({'name': 'a', 'output': '/x/y.3tz'}))
        self.assertIsNone(parsed['recipes'][0]['convert'])

    def test_substitute(self):
        argv = fly.substitute(['vector', '-i', '{corpus}/v.geojson', '-o', '{out}.3tz'], pathlib.Path('/o/k'), pathlib.Path('/c'))
        self.assertEqual(argv, ['vector', '-i', '/c/v.geojson', '-o', '/o/k.3tz'])

    def test_invalid_recipes(self):
        good = {'name': 'a', 'convert': ['x', '{out}']}
        bad = [
            dict(set='s', dataset='d', recipes=[]),
            dict(set='Bad Name', dataset='d', recipes=[good]),
            dict(set='s', recipes=[good]),
            dict(set='s', dataset='d', recipes=[good], extra=1),
            self.doc(good, good),
            self.doc({'name': 'a', 'convert': ['x']}),  # no {out}
            self.doc({'name': 'a', 'convert': ['x', '{out}', '{input}']}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'output': '/y'}),
            self.doc({'name': 'a'}),
            self.doc({'name': 'b', 'like': 'a'}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'fov': 1}}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'maximumScreenSpaceError': -1}}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'maximumScreenSpaceError': True}}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'tilesetOptions': []}}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'viewer': {'scheduler': {'priority': 1}}}),
            self.doc({'name': 'a', 'convert': ['x', '{out}'], 'leaf': 3}),
        ]
        for item in bad:
            with self.subTest(item=item), self.assertRaises(ValueError): fly.parse_recipes(item)

    def test_shipped_recipes(self):
        sets = fly.load_recipe_sets(sorted((HERE / 'recipes').glob('*.json')))
        names = {r['name']: r for s in sets for r in s['recipes']}
        self.assertEqual(names['terrain-skiplod']['viewer'], {'tilesetOptions': {'skipLevelOfDetail': True}})
        self.assertEqual(names['terrain-skiplod']['convert'], names['terrain-lod']['convert'])
        self.assertIn('vector-leaf16', names)

    def test_names_unique_across_sets(self):
        files = sorted((HERE / 'recipes').glob('*.json'))
        with self.assertRaises(ValueError): fly.load_recipe_sets(files[:1] * 2)


class Stats(unittest.TestCase):
    def test_percentile_matches_linear_interpolation(self):
        data = [15, 20, 35, 40, 50]
        self.assertEqual(rep.percentile(data, 0), 15)
        self.assertEqual(rep.percentile(data, 50), 35)
        self.assertAlmostEqual(rep.percentile(data, 40), 29)  # position 1.6 -> 20 + 0.6 * 15
        self.assertAlmostEqual(rep.percentile(data, 95), 48)
        self.assertEqual(rep.percentile(data, 100), 50)
        self.assertEqual(rep.percentile([7], 99), 7)
        self.assertIsNone(rep.percentile([], 50))
        self.assertEqual(rep.percentile([3, None, 1], 50), 2)
        self.assertAlmostEqual(rep.percentile(list(range(1, 101)), 99), 99.01)

    def test_intervals_follow_the_mask(self):
        times = [0, 16, 32, 132, 148]
        self.assertEqual(rep.intervals(times), [16, 16, 100, 16])
        self.assertEqual(rep.intervals(times, [1, 1, 0, 0, 1]), [16, 16])

    def test_basis_is_motion_else_hold_without_cuts(self):
        frames = dict(t=[0] * 5, hold=[1, 1, 0, 0, 1], gap=[0, 0, 0, 1, 0])
        self.assertEqual(rep.basis(frames, [dict(frame=0), dict(frame=4)]), ('motion', [False, False, True, False, False]))
        teleport = dict(t=[0] * 6, hold=[1] * 6, gap=[0, 0, 0, 1, 0, 0])
        self.assertEqual(rep.basis(teleport, [dict(frame=0), dict(frame=3)]), ('hold', [False, True, True, False, True, True]))

    def test_hitches_are_strictly_above(self):
        values = [16.7, 33, 33.1, 100, 100.5]
        self.assertEqual(rep.hitches(values, 33), 3)
        self.assertEqual(rep.hitches(values, 100), 1)

    def test_peak_rate(self):
        times = [0, 100, 500, 999, 1000, 1500, 2600]
        counts = [1, 1, 1, 1, 5, 0, 1]
        self.assertEqual(rep.peak_rate(times, counts), 8)  # [100, 1000] spans < 1 s and holds 1 + 1 + 1 + 5
        self.assertEqual(rep.peak_rate(times, counts, window_ms=500), 12)  # 999 and 1000 hold 6 in under 0.5 s
        self.assertEqual(rep.peak_rate([], []), 0)

    def test_pops(self):
        selected = [10, 10, 12, 30, 30, 2]
        added = [10, 1, 3, 20, 0, 0]
        removed = [0, 1, 1, 2, 0, 28]
        # frame 1: churn 2 < 5; frame 2: 4 < 5; frame 3: 22 >= max(4, 6); frame 5: 28 >= 15
        self.assertEqual(rep.pops(selected, added, removed), 2)
        self.assertEqual(rep.pops([0, 5], [0, 5], [0, 0]), 0)  # nothing selected before: first selection is not a pop
        self.assertEqual(rep.pops(selected, added, removed, [1, 1, 1, 0, 1, 1]), 1)  # frame 3 masked out

    def test_sse_jumps(self):
        self.assertEqual(rep.sse_jumps([20, 18, 6, 6, 0, 30, 14, 4]), 3)  # 18->6, 30->14, 14->4; 0 is no selection
        self.assertEqual(rep.sse_jumps([20, 18, 6, 6, 0, 30, 14, 4], [1, 1, 0, 1, 1, 1, 1, 0]), 1)  # only 30->14

    def summary_run(self):
        return dict(firstRenderSeconds=0.5,
                    waypoints=[dict(name='a', reached=True, fullQualitySeconds=0.4), dict(name='b', reached=True, fullQualitySeconds=0.1)],
                    frames=dict(t=[0, 16, 32, 82, 98], cpu=[5, 1, 1, 1, 1], gpu=[None, 1, 2, 3, None], hold=[1, 0, 0, 0, 1],
                                gap=[0, 0, 0, 0, 0], visited=[1, 2, 3, 3, 3], selected=[0, 1, 2, 2, 2],
                                drawCalls=[1, 2, 3, 3, 3], triangles=[0, 10, 20, 20, 20], geometryBytes=[0, 1, 2, 2, 2],
                                textureBytes=[0, 0, 1, 1, 1], memoryBytes=[0, 1, 3, 3, 3], loads=[0, 1, 1, 0, 0],
                                unloads=[0, 0, 0, 1, 0], reloads=[0, 0, 0, 0, 0], requests=[1, 1, 0, 0, 0],
                                requestBytes=[100, 200, 0, 0, 0], selAdded=[0, 1, 1, 0, 0], selRemoved=[0, 0, 0, 0, 0],
                                maxSse=[0, 10, 12, 12, 12], memorySse=[16] * 5))

    def test_summarize(self):
        s = rep.summarize(self.summary_run())
        self.assertEqual(s['frames'], 5)
        self.assertEqual((s['frameBasis'], s['basisFrames']), ('motion', 3))
        self.assertEqual(s['hitches33'], 1)  # motion intervals 16, 16, 50
        self.assertEqual(s['frameP50Ms'], 16)
        self.assertEqual(s['allHitches33'], 1)  # plus the 16 ms hold frame at the end
        self.assertEqual(s['cpuP50Ms'], 1)  # the 5 ms first (hold) frame is outside the basis
        self.assertAlmostEqual(s['fullQualitySeconds'], 0.5)
        self.assertEqual(s['gpuTimedFrames'], 3)  # motion frames 1-3 all timed
        self.assertEqual((s['loads'], s['evictions'], s['requests']), (2, 1, 2))
        self.assertEqual(s['peakRequestsPerSecond'], 2)
        run = self.summary_run(); run['waypoints'][1]['reached'] = False
        self.assertIsNone(rep.summarize(run)['fullQualitySeconds'])
        self.assertEqual(rep.summarize(run)['unreachedWaypoints'], 1)


    def test_render_keeps_profiles_apart_and_marks_captures(self):
        summary = rep.summarize(self.summary_run())
        host = dict(wallSeconds=10, psiStart={}, psiEnd={}, memAvailableMinGiB=30, gpuUtilStart=0, cpuBusyMean=0.1, loadAverage=1)
        facts, noise = rep.host_noise(host)
        runs = [dict(id=f'r.p.{profile}.r{i}', recipe='r', set='s', path='p', profile=profile, repeat=i, untimed=i == 0, trace=dict(mainBusyMs=12.0, jsMs=5.0) if i == 0 else None,
                     hostFacts=facts, noise=noise, problems=[], summary=summary) for profile in ('native', 'cpu4x') for i in (0, 1, 2)]
        aggregates = {f'r|p|{profile}': dict(rep.aggregate([summary, summary], [False, False]),
                                              waypoints={'a': dict(median=0.4, n=2, of=2, unreached=0)})
                      for profile in ('native', 'cpu4x')}
        artifact = dict(generated='now', runs=runs, aggregates=aggregates,
                        screenshots={'r|p|native|a': dict(file='screenshots/x.png', sha256='ab' * 32)},
                        environment=dict(evidence='/e', commit='c', binary=dict(version='v', sha256='0' * 64), renderer='GPU',
                                         rendererClass='gpu', browserVersion='b', browser=dict(mode='cdp'), viewport=[1024, 768],
                                         devicePixelRatio=1, cesium='1.146.0', fps=60, medianIntervalMs=16.7, gpuTimer=None,
                                         repeats=2, settleMs=250, holdSeconds=10, matrix=dict(name='validate')),
                        paths={'p': dict(description='d', motionFrames=3, waypoints=['a', 'b'])},
                        profiles={'native': {'description': 'n'}, 'cpu4x': {'cpuThrottlingRate': 4}},
                        recipes=[dict(set='s', name='r', convert=['x'], viewer={}, output=dict(bytes=1, files=1))])
        text = rep.render(artifact, pathlib.Path('/e'), pathlib.Path('/e/FLYTHROUGH.md'))
        self.assertIn('## s: native', text)
        self.assertIn('## s: cpu4x', text)
        self.assertIn('| p | r | motion | 16.0 / ', text)
        self.assertIn('| 2/2 (2 clean) |', text)
        self.assertIn('| native | 2 | 0 |', text)  # capture runs are not counted as timed runs
        self.assertIn('`abababab', text)
        self.assertIn('| r.p.native.r0 | 12 | 5 |', text)


class Problems(unittest.TestCase):
    def test_cancelled_tile_requests_are_not_failures(self):
        renderer = dict(renderer='GPU')
        base = dict(loaded=True, renderer='GPU', visibility=dict(after='visible'))
        cancelled = dict(url='http://127.0.0.1:1/data/t/a.glb', error='net::ERR_ABORTED')
        self.assertEqual(fly.problems(dict(base, failedRequests=[cancelled]), renderer), [])
        for bad in (dict(url='http://127.0.0.1:1/data/t/a.glb', status=404),
                    dict(url='http://127.0.0.1:1/cesium/Cesium.js', error='net::ERR_ABORTED'),
                    dict(url='http://127.0.0.1:1/data/t/a.glb', error='net::ERR_CONNECTION_RESET')):
            with self.subTest(bad=bad):
                self.assertEqual(len(fly.problems(dict(base, failedRequests=[cancelled, bad]), renderer)), 1)
        self.assertEqual(fly.problems(dict(base, renderer='SwiftShader'), renderer), ["renderer changed: 'SwiftShader'"])


class Trace(unittest.TestCase):
    def test_union_counts_overlaps_once(self):
        self.assertEqual(rep.union_ms([(0, 1000), (500, 1500), (3000, 4000), (3100, 3200)]), 2.5)
        self.assertEqual(rep.union_ms([]), 0)

    def test_summary_splits_roles_by_page_process(self):
        url = 'http://127.0.0.1:1234/'
        events = [
            dict(ph='M', name='process_name', pid=2, args=dict(name='GPU Process')),
            dict(ph='M', name='thread_name', pid=1, tid=10, args=dict(name='CrRendererMain')),
            dict(ph='M', name='thread_name', pid=1, tid=11, args=dict(name='DedicatedWorker thread')),
            dict(ph='M', name='thread_name', pid=2, tid=20, args=dict(name='CrGpuMain')),
            dict(ph='M', name='thread_name', pid=9, tid=90, args=dict(name='CrRendererMain')),
            dict(ph='I', name='TracingStartedInBrowser', pid=0, tid=0, ts=0,
                 args=dict(data=dict(frames=[dict(processId=1, url=url), dict(processId=9, url='http://other/')]))),
            dict(ph='X', name='RunTask', cat='toplevel', pid=1, tid=10, ts=0, dur=10000),
            dict(ph='X', name='FireAnimationFrame', cat='devtools.timeline', pid=1, tid=10, ts=1000, dur=6000),
            dict(ph='X', name='FunctionCall', cat='devtools.timeline', pid=1, tid=10, ts=2000, dur=1000),  # nested: once
            dict(ph='X', name='MinorGC', cat='v8', pid=1, tid=10, ts=8000, dur=500),
            dict(ph='X', name='CommandBufferProxyImpl::WaitForGetOffsetInRange', cat='gpu', pid=1, tid=10, ts=9000, dur=800),
            dict(ph='X', name='RunTask', cat='toplevel', pid=1, tid=11, ts=0, dur=4000),
            dict(ph='X', name='RunTask', cat='toplevel', pid=2, tid=20, ts=0, dur=3000),
            dict(ph='X', name='RunTask', cat='toplevel', pid=9, tid=90, ts=0, dur=50000),  # another tab: ignored
        ]
        out = rep.trace_summary(events, url)
        self.assertEqual(out['rendererPids'], [1])
        self.assertEqual((out['mainBusyMs'], out['jsMs'], out['gcMs']), (10, 6, 0.5))
        self.assertEqual((out['gpuCommandsMs'], out['gpuWaitMs']), (0.8, 0.8))
        self.assertEqual((out['workerBusyMs'], out['workerThreads'], out['gpuProcessBusyMs']), (4, 1, 3))


class Noise(unittest.TestCase):
    def host(self, **extra):
        base = dict(wallSeconds=10, psiStart=dict(cpu=0, memory=0, io=0), psiEnd=dict(cpu=1e6, memory=0, io=0),
                    memAvailableMinGiB=30, gpuUtilStart=5, cpuBusyMean=0.2, loadAverage=1)
        base.update(extra)
        return base

    def test_clean_and_noisy(self):
        facts, reasons = rep.host_noise(self.host())
        self.assertAlmostEqual(facts['cpuPressure'], 0.1)
        self.assertEqual(reasons, [])
        _, reasons = rep.host_noise(self.host(psiEnd=dict(cpu=3e6, memory=5e5, io=0), memAvailableMinGiB=2, gpuUtilStart=60))
        self.assertEqual(len(reasons), 4)
        _, reasons = rep.host_noise(self.host(psiStart={}, psiEnd={}, gpuUtilStart=None))
        self.assertEqual(reasons, [])  # missing facts are not evidence of noise

    def test_aggregate_prefers_clean_runs(self):
        summaries = [dict(frameP95Ms=17.0, requests=10), dict(frameP95Ms=17.5, requests=10), dict(frameP95Ms=40.0, requests=12)]
        out = rep.aggregate(summaries, [False, False, True])
        self.assertEqual((out['runs'], out['clean'], out['usedNoisy']), (3, 2, False))
        self.assertEqual(out['metrics']['frameP95Ms']['median'], 17.25)
        self.assertTrue(out['metrics']['frameP95Ms']['stable'])
        self.assertEqual((out['metrics']['frameP95Ms']['n'], out['metrics']['frameP95Ms']['of']), (2, 2))
        out = rep.aggregate(summaries, [False, True, True])  # one clean run: use all, and say so
        self.assertTrue(out['usedNoisy'])
        self.assertEqual(out['metrics']['frameP95Ms']['median'], 17.5)
        self.assertFalse(out['metrics']['frameP95Ms']['stable'])  # 23 ms spread > max(1.75, 1)
        self.assertTrue(out['metrics']['requests']['stable'])  # 2 <= count floor

    def test_single_value_has_unknown_stability(self):
        out = rep.aggregate([dict(fullQualitySeconds=None, requests=3), dict(fullQualitySeconds=1.0, requests=3)], [False, False])
        self.assertEqual(out['metrics']['fullQualitySeconds'], dict(median=1.0, min=1.0, max=1.0, n=1, of=2, stable=None))
        self.assertTrue(out['metrics']['requests']['stable'])

    def test_floors(self):
        self.assertEqual(rep.floor_for('frameP95Ms'), 1.0)
        self.assertEqual(rep.floor_for('fullQualitySeconds'), 0.05)
        self.assertEqual(rep.floor_for('requestMiB'), 0.25)
        self.assertEqual(rep.floor_for('lodPops'), rep.COUNT_FLOOR)


@unittest.skipUnless(shutil.which('node'), 'node is required to load flythrough.cjs')
class Driver(unittest.TestCase):
    def test_driver_loads_and_exports(self):
        script = f'const m = require({json.dumps(str(HERE / "flythrough.cjs"))}); console.log(typeof m.fly);'
        self.assertEqual(subprocess.run(['node', '-e', script], capture_output=True, text=True, check=True).stdout.strip(), 'function')


if __name__ == '__main__':
    unittest.main()
