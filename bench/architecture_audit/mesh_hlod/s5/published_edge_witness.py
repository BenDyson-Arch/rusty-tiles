"""Price one original edge point against a complete delivered root GLB, exactly."""
import hashlib
import json
import sys
from fractions import Fraction
from pathlib import Path

from archive import Archive, expand
from oracle import distance2
from spatial import f32_point


def check(archive, witness):
    with Archive(archive) as data:
        roots = []
        for tile in expand(data):
            if not tile['contents']:
                continue
            parent = tile['parent']
            while parent is not None and not parent['contents']:
                parent = parent['parent']
            if parent is None:
                roots.append(tile)
        assert len(roots) == 1 and len(roots[0]['contents']) == 1, 'expected one rendered root'
        payload = data.read(roots[0]['contents'][0])
        assert hashlib.sha256(payload).hexdigest() == witness['sha256']
        g = Fraction(roots[0]['g'])
    a, b = map(f32_point, witness['edge'])
    t = Fraction(*witness['fraction'])
    assert 0 < t < 1 and t != Fraction(1, 2)
    p = [(1-t)*Fraction(x) + t*Fraction(y) for x, y in zip(a, b)]
    squared = min(distance2(p, tuple(map(f32_point, face))) for face in witness['decoded'])
    # Require an actual gap at the chosen interior point even if the generator changes.
    assert squared > Fraction(1, 4), squared
    print(f'published G {float(g):.17g}, exact edge distance squared {squared}', flush=True)
    assert g*g >= squared, 'published root G understates the original edge interior'


if __name__ == '__main__':
    check(Path(sys.argv[1]), json.loads(Path(sys.argv[2]).read_text()))
