"""Analytic positives, exact violating witnesses, and unresolved work."""
import json
from fractions import Fraction as Q
from oracle import Failure, distance2, require
from spatial import Store, Work, WorkLimit
from surface_error import directed, exact, inspect


def rect(a, b):
    h = Q(1, 16384)
    x,y,z,w = (a,0,0),(b,0,0),(b,h,0),(a,h,0)
    return [exact((x,y,z)),exact((x,z,w))]


def verify():
    source = rect(0,1)
    proxy = rect(0,Q(1,4)) + rect(Q(3,8),1)
    # The entire edge strip is contained in the 1/16 offsets. At its centre
    # the exact nearest distance realizes that value in the forward direction.
    g = Q(1,16)
    witness = (Q(5,16),Q(0),Q(0))
    assert min(distance2(witness,f) for f in proxy) == g*g
    for a,b in [(source,proxy),(proxy,source)]:
        assert directed(a,b,g,Work(),distance2,require,'strip') >= len(a)
    for a,b in [(source,proxy),(proxy,source)]:
        # Both orientations run for a symmetric claim. A reverse-only pass
        # would incorrectly accept the hole in one of these calls.
        try:
            for fr,to in [(a,b),(b,a)]:
                directed(fr,to,g/2,Work(),distance2,require,'strip mutant')
        except Failure as e:
            assert e.check == 'sourceSurfaceError'
        else:
            raise AssertionError('missed original edge interior')
    try:
        directed(source,proxy,g,Work(1),distance2,require,'budget')
    except WorkLimit:
        pass
    else:
        raise AssertionError('work exhaustion accepted')
    for f in [exact(((0,0,0),)*3),exact(((0,0,0),(1,0,0),(1,0,0)))]:
        assert directed([f],[f],0,Work(),distance2,require,'degenerate')==1


def welded_sliver():
    height = Q(1, 32)
    base = exact(((0,0,0),(1,0,0),(1,0,1)))
    sliver = exact(((0,0,0),(0,height,0),(1,0,0)))
    source, decoded = [base,sliver], [base]
    assert directed(source,decoded,height,Work(),distance2,require,'original sliver') >= 2
    assert directed(decoded,source,0,Work(),distance2,require,'decoded sheet') == 1
    try:
        directed(source,decoded,height/2,Work(),distance2,require,'sliver mutant')
    except Failure as error:
        assert error.check == 'sourceSurfaceError'
    else:
        raise AssertionError('lost original sliver accepted')


def source_join():
    # The six original and decoded extrema agree. The complete comparison still
    # finds a missing interior once G falls below the half-width of the strip.
    origin = rect(0, 1)
    decoded = rect(0, Q(1, 4)) + rect(Q(3, 8), 1)
    def payload(face):
        return {'xyz': [[float(v) for v in p] for p in face]}
    leaf = lambda i: dict(id=i+1, lo=i, hi=i+1, children=[], rendered=True, relative=())
    children = [leaf(0), leaf(1)]
    root = dict(id=0, lo=0, hi=2, children=children, rendered=True, relative=(), g=1/16)
    tiles = [root, *children]
    db = Store(':memory:')
    try:
        for i, face in enumerate(origin):
            db.db.execute('INSERT INTO source(id, leaf, data) VALUES (?,?,?)',
                          (i, i, json.dumps(payload(face))))
        db.put_faces(0, [payload(f) for f in decoded])
        checked = inspect(db, tiles, True, distance2, require, ())
        assert checked['completeTwoSidedTiles'] == checked['extremaTiles'] == 1
        root['g'] /= 2
        assert inspect(db, tiles, False, distance2, require, ())['extremaTiles'] == 1
        try:
            inspect(db, tiles, True, distance2, require, ())
        except Failure as error:
            assert error.check == 'sourceSurfaceError'
        else:
            raise AssertionError('source ownership join accepted an understated interior gap')
    finally:
        db.close()


if __name__=='__main__':
    verify()
    welded_sliver()
    source_join()
    print('exact closed-strip, reverse-only, degenerate and work-limit controls passed')
