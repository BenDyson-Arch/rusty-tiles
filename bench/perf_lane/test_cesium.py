"""Unit tests for cesium.py's renderer check and gate, and cesium.cjs's finish rule (run under node).

Run: python3 -I bench/perf_lane/test_cesium.py
"""
import json, pathlib, shutil, subprocess, sys, tempfile, unittest

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


class Renderer(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
