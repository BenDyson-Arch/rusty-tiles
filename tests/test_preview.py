"""The preview must expose only explicitly selected output directories."""
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('preview', ROOT/'scripts/preview.py')
preview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)


class PreviewTests(unittest.TestCase):
    def test_explicit_roots_and_no_directory_listing(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            selected = base/'selected'
            selected.mkdir()
            manifest = selected/'tileset.json'
            manifest.write_text('{}')
            secret = base/'private.txt'
            secret.write_text('not selected')
            cls = preview.make_handler(base, {'annotations': selected})
            handler = cls.__new__(cls)
            self.assertEqual(pathlib.Path(handler.translate_path('/annotations/tileset.json')), manifest)
            self.assertEqual(pathlib.Path(handler.translate_path('/')), ROOT/'preview/index.html')
            for path in ['/annotations/', '/data/private.txt', '/private.txt', '/annotations/../private.txt', '/annotations/%2e%2e/private.txt']:
                self.assertFalse(pathlib.Path(handler.translate_path(path)).is_file(), path)

    def test_symlink_cannot_escape_selected_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            selected = base/'selected'
            selected.mkdir()
            secret = base/'private.txt'
            secret.write_text('not selected')
            (selected/'escape.txt').symlink_to(secret)
            cls = preview.make_handler(base, {'mesh': selected})
            handler = cls.__new__(cls)
            self.assertFalse(pathlib.Path(handler.translate_path('/mesh/escape.txt')).exists())

    def test_no_implicit_data_or_missing_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            (base/'Cesium.js').write_text('// fixture')
            for extra in [[], ['--mesh', tmp]]:
                result = subprocess.run([sys.executable, str(ROOT/'scripts/preview.py'), '--cesium', tmp, *extra], capture_output=True)
                self.assertEqual(result.returncode, 2)


if __name__ == '__main__':
    unittest.main()
