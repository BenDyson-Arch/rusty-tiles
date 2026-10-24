"""Unit tests for cesium.py's renderer check and gate, and cesium.cjs's finish rule (run under node).

Run: python3 -I bench/perf_lane/test_cesium.py
"""
import contextlib, io, json, pathlib, shutil, subprocess, sys, tempfile, unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import cesium  # noqa: E402

NVIDIA = 'ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 4080 SUPER/PCIe/SSE2, OpenGL ES 3.2)'
SWIFTSHADER = 'ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)'


def finish(window_ms, frames):
    """Feed (ready, now) frames to cesium.cjs's finishTracker; returns the per-frame results."""
    script = (f'const {{finishTracker}} = require({json.dumps(str(HERE / "cesium.cjs"))});'
              f'const t = finishTracker({window_ms});'
              f'console.log(JSON.stringify({json.dumps(frames)}.map(([ready, now]) => t.observe(ready, now))));')
    return json.loads(subprocess.run(['node', '-e', script], capture_output=True, text=True, check=True).stdout)


@unittest.skipUnless(shutil.which('node'), 'node is required for the cesium.cjs finish rule')
class FinishRule(unittest.TestCase):
    def test_done_after_window_of_wall_time_reports_onset(self):
        frames = [(True, i * 16) for i in range(20)]  # 60 Hz, ready from the first frame
        results = finish(250, frames)
        first = next(i for i, r in enumerate(results) if r)
        self.assertEqual(frames[first][1], 256)  # the first frame at least 250 ms after the onset
        self.assertEqual(results[first], dict(at=0, frame=1))  # the reported time is the onset, not the confirmation

    def test_not_ready_restarts_the_window(self):
        frames = [(True, 0), (True, 100), (False, 200), (True, 216), (True, 400), (True, 466), (True, 470)]
        results = finish(250, frames)
        self.assertEqual(results[:5], [None] * 5)
        self.assertEqual(results[5], dict(at=216, frame=4))

    def test_wall_time_not_frame_count(self):
        # Five quick ready frames (the old rule's threshold) are not enough...
        self.assertEqual(finish(250, [(True, i * 10) for i in range(6)]), [None] * 6)
        # ...while two slow frames spanning the window are: costly frames add no settle tail beyond the window.
        self.assertEqual(finish(250, [(True, 0), (True, 3000)]), [None, dict(at=0, frame=1)])

    def test_never_ready_and_zero_window(self):
        self.assertEqual(finish(250, [(False, i * 100) for i in range(10)]), [None] * 10)
        self.assertEqual(finish(0, [(False, 0), (True, 16)]), [None, dict(at=16, frame=2)])


def settle(frames, *, timeout=False):
    """Run the actual settle closure with deterministic render events and a controllable timer."""
    source = (HERE / 'cesium.cjs').read_text()
    closure = source.split('  const settle =', 1)[1].split('  const seconds =', 1)[0]
    script = f'''
const {{finishTracker}} = require({json.dumps(str(HERE / 'cesium.cjs'))});
const spec = {{settleMs: 250, stepMs: 1000}}, failures = [];
let now = 0, ready = false, inflight = 0, active = 0;
let listener, timer, requests = 0, removed = 0, cleared = 0;
const performance = {{now: () => now}};
const setTimeout = (callback, ms) => {{
  if (ms !== spec.stepMs) throw Error('wrong timeout');
  timer = callback; return 1;
}};
const clearTimeout = id => {{
  if (id !== 1) throw Error('wrong timer');
  cleared++; timer = null;
}};
const scene = {{postRender: {{addEventListener: callback => {{
  listener = callback;
  return () => {{removed++; listener = null;}};
}}}}, requestRender: () => requests++}};
const idle = () => inflight === 0 && active === 0;
const settle = {closure}
(async () => {{
  const pending = settle(() => ready);
  for (const frame of {json.dumps(frames)}) {{
    [now, ready, inflight, active] = frame;
    if (frame[4]) failures.push('render failure');
    if (!listener) throw Error('listener removed prematurely');
    listener();
  }}
  if ({json.dumps(timeout)}) timer();
  const result = await pending;
  console.log(JSON.stringify({{result, requests, removed, cleared, listening: !!listener}}));
}})();
'''
    return json.loads(subprocess.run(['node', '-e', script], capture_output=True, text=True, check=True,
                                     timeout=5).stdout)


@unittest.skipUnless(shutil.which('node'), 'node is required for the cesium.cjs settle closure')
class Settle(unittest.TestCase):
    def test_requests_must_stay_idle_for_the_whole_window(self):
        result = settle([[0, True, 0, 0], [100, True, 1, 0], [150, True, 0, 0],
                         [300, True, 0, 1], [350, True, 0, 0], [599, True, 0, 0], [600, True, 0, 0]])
        self.assertEqual(result, dict(result=dict(loaded=True, at=350, frames=5, confirmedAt=600),
                                      requests=7, removed=1, cleared=1, listening=False))

    def test_failure_overrides_completion_and_cleans_up(self):
        result = settle([[0, True, 0, 0], [250, True, 0, 0, True]])
        self.assertEqual(result, dict(result=dict(loaded=False), requests=2, removed=1, cleared=1, listening=False))

    def test_timeout_removes_listener_without_reporting_loaded(self):
        result = settle([[0, False, 0, 0], [100, False, 0, 0]], timeout=True)
        self.assertEqual(result, dict(result=dict(loaded=False), requests=3, removed=1, cleared=0, listening=False))


class Renderer(unittest.TestCase):
    def test_preflight_timeout_fails_cleanly_and_releases_lock(self):
        args = type('Args', (), dict(launch=True, software_gl=False, angle='gl', chromium='chrome'))
        cesium_lock = cesium.browser_lock
        with tempfile.TemporaryDirectory() as tmp:
            lock = pathlib.Path(tmp) / 'lock'
            with mock.patch.object(cesium, 'browser_lock', side_effect=lambda: cesium_lock(lock)), \
                    mock.patch.object(cesium.subprocess, 'run', side_effect=subprocess.TimeoutExpired('node', 30)), \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                with self.assertRaises(SystemExit) as raised:
                    cesium.preflight(args, {})
            self.assertEqual(raised.exception.code, cesium.ENVIRONMENT_EXIT)
            self.assertEqual(stderr.getvalue(), f'cesium: browser preflight timed out after {cesium.STARTUP_SECONDS:g} s\n')
            # A second independently opened descriptor must be able to acquire the released lock.
            with open(lock, 'a') as handle:
                cesium.fcntl.flock(handle, cesium.fcntl.LOCK_EX | cesium.fcntl.LOCK_NB)

    def test_renderer_class(self):
        self.assertEqual(cesium.renderer_class(NVIDIA), 'gpu')
        self.assertEqual(cesium.renderer_class('ANGLE (AMD, AMD Radeon RX 7900 XTX (radeonsi, navi31, LLVM 19.1.7), OpenGL 4.6)'),
                         'gpu')  # Mesa's LLVM shader compiler is not llvmpipe
        self.assertEqual(cesium.renderer_class(SWIFTSHADER), 'software')
        self.assertEqual(cesium.renderer_class('SwiftShader Device (Subzero)'), 'software')
        self.assertEqual(cesium.renderer_class('ANGLE (Mesa, llvmpipe (LLVM 19.1.7, 256 bits), OpenGL 4.5)'), 'software')
        self.assertEqual(cesium.renderer_class('Microsoft Basic Render Driver'), 'software')
        self.assertEqual(cesium.renderer_class(''), 'unknown')
        self.assertEqual(cesium.renderer_class(None), 'unknown')

    def test_mode_check(self):
        self.assertIsNone(cesium.check_renderer_class(NVIDIA, software_gl=False))
        self.assertIsNone(cesium.check_renderer_class(SWIFTSHADER, software_gl=True))
        self.assertIn('pass --software-gl', cesium.check_renderer_class(SWIFTSHADER, software_gl=False))
        self.assertIn('is not a GPU', cesium.check_renderer_class(None, software_gl=False))
        self.assertIn('--software-gl was given', cesium.check_renderer_class(NVIDIA, software_gl=True))

    def test_browser_spec(self):
        defaults = dict(launch=False, software_gl=False, angle='gl', chromium='chrome', cdp_url='http://localhost:9222')
        args = lambda **kw: type('Args', (), dict(defaults, **kw))
        self.assertEqual(cesium.browser_spec(args()), dict(mode='cdp', cdpUrl='http://localhost:9222'))
        self.assertEqual(cesium.browser_spec(args(launch=True))['gl'], 'gl')
        self.assertEqual(cesium.browser_spec(args(software_gl=True)), dict(mode='launch', gl='swiftshader',
                                                                           executablePath='chrome'))

    def test_unreachable_endpoint_fails_clearly(self):
        with self.assertRaises(SystemExit) as raised: cesium.cdp_version('http://127.0.0.1:9')
        self.assertEqual(raised.exception.code, cesium.ENVIRONMENT_EXIT)

    def test_browser_lock_is_reentrant_across_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / 'lock'
            for _ in range(2):
                with cesium.browser_lock(path): self.assertTrue(path.exists())


def case(**changes):
    base = dict(loaded=True, renderer=NVIDIA, rendererClass='gpu', secondsToLoaded=1.0, console=dict(errors=[]),
                requests=dict(requests=10, requestBytes=1000, serverErrors=[]))
    return dict(base, **changes)


class Gate(unittest.TestCase):
    GPU = dict(renderer=NVIDIA, rendererClass='gpu')

    def test_failures_always_gate(self):
        self.assertEqual(cesium.problems(case(), None, self.GPU), [])
        self.assertEqual(cesium.problems(case(loaded=False), None, self.GPU), ['load failed or timed out'])
        self.assertEqual(cesium.problems(case(console=dict(errors=['x'])), None, self.GPU), ['console errors: 1'])

    def test_renderer_change_is_a_problem(self):
        [found] = cesium.problems(case(renderer=SWIFTSHADER), None, self.GPU)
        self.assertIn('renderer changed during the run', found)

    def test_timing_compared_only_within_a_renderer_class(self):
        slow = case(secondsToLoaded=2.0)
        self.assertEqual(len(cesium.problems(slow, case(), self.GPU)), 1)  # +100%, +1 s: flagged
        self.assertEqual(cesium.problems(slow, case(rendererClass='software'), self.GPU), [])
        self.assertEqual(cesium.problems(case(secondsToLoaded=1.2), case(), self.GPU), [])  # under 1.25x
        # Requests are renderer-independent and still compared.
        more = case(requests=dict(requests=20, requestBytes=1000, serverErrors=[]))
        self.assertEqual(len(cesium.problems(more, case(rendererClass='software'), self.GPU)), 1)


class TilesetOptions(unittest.TestCase):
    KINDS = dict(terrain='tileset', raster='raster')

    def test_parses_a_tileset_case(self):
        self.assertEqual(cesium.parse_tileset_options(['terrain={"skipLevelOfDetail":true}'], self.KINDS),
                         dict(terrain=dict(skipLevelOfDetail=True)))
        self.assertEqual(cesium.parse_tileset_options([], self.KINDS), {})

    def test_rejects_what_it_would_ignore(self):
        for item in ('missing={"a":1}', 'raster={"a":1}', 'terrain=true', 'terrain=[1]', 'terrain=nope', 'terrain'):
            with self.subTest(item=item), self.assertRaises(ValueError):
                cesium.parse_tileset_options([item], self.KINDS)
        with self.assertRaises(ValueError):
            cesium.parse_tileset_options(['terrain={}', 'terrain={"a":1}'], self.KINDS)

    def test_spec_reaches_the_page(self):
        script = (HERE / 'cesium.cjs').read_text()
        self.assertIn('...(spec.tilesetOptions || {}), maximumScreenSpaceError', script)


if __name__ == '__main__':
    unittest.main()
