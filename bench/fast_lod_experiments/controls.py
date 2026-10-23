"""Calibrate the independent distance and colour evaluator with analytic answers."""
import tempfile,pathlib,subprocess,numpy as np
with tempfile.TemporaryDirectory() as t:
 d=pathlib.Path(t);np.array([[[0,0,0],[1,0,0],[0,1,0]]],np.float64).tofile(d/'t')
 np.array([[.25,.25,2],[2,0,0],[-1,0,0]],np.float64).tofile(d/'q')
 subprocess.run(['/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024/nearest',str(d/'t'),str(d/'q'),str(d/'r')],check=True)
 r=np.fromfile(d/'r',np.float64).reshape(-1,5)
 assert np.allclose(r[:,0],[2,1,1],rtol=0,atol=1e-14)
 assert np.allclose(r[:,2:],[[.5,.25,.25],[0,1,0],[1,0,0]],rtol=0,atol=1e-14)
 # Sensitive geometry mutant: 1m surface displacement must be visible.
 assert r[0,0] > 1.9
print('Independent distance: analytic interior, edge, vertex and displacement controls passed.')

from legacy_seams import canonical_vertices
# Coordinate-wise sorting falsely accepts these different triangles.
a=np.array([[[0,0,0],[1,2,0],[2,1,0]]],dtype=float)
b=np.array([[[0,1,0],[1,0,0],[2,2,0]]],dtype=float)
assert np.array_equal(np.sort(a,axis=1),np.sort(b,axis=1))
assert not np.array_equal(canonical_vertices(a),canonical_vertices(b))
assert np.array_equal(canonical_vertices(a),canonical_vertices(a[:,[1,2,0],:]))
print('Legacy corner identity: cyclic permutation accepted, adversarial reassociated XYZ rejected.')
