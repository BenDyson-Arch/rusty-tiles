"""The rusty-tiles CLI selected for Python acceptance tests.

RUSTY_TILES_BIN names the binary. Without it CLI tests are skipped after one
warning, unless RUSTY_TILES_REQUIRE_BIN=1 (set in CI), which makes every
importing test module fail instead of passing by skipping.
"""
import os
import pathlib
import sys
import unittest

BIN = os.environ.get('RUSTY_TILES_BIN') or None
REQUIRED = os.environ.get('RUSTY_TILES_REQUIRE_BIN') == '1'

if BIN is None:
    if REQUIRED:
        raise RuntimeError('RUSTY_TILES_REQUIRE_BIN=1 but RUSTY_TILES_BIN is not set; '
                           'set it to the absolute path of the rusty-tiles binary')
    print('WARNING: RUSTY_TILES_BIN is not set; native CLI tests will be SKIPPED. '
          'Build target/debug/rusty-tiles and set RUSTY_TILES_BIN to its absolute path.', file=sys.stderr)
elif not pathlib.Path(BIN).is_file():
    raise RuntimeError(f'RUSTY_TILES_BIN does not name a file: {BIN}')


def requires_bin(reason='set RUSTY_TILES_BIN for native CLI acceptance'):
    """Skip a test or class when no CLI is selected (never under RUSTY_TILES_REQUIRE_BIN=1)."""
    return unittest.skipUnless(BIN, reason)
