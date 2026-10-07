"""Native CLI terrain checked against the original Python/GDAL development oracle."""
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import types
import unittest

import numpy as np
from osgeo import gdal, osr
from test_derivatives import decode_terrain

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = os.environ.get('RUSTY_TILES_BIN')
spec = importlib.util.spec_from_file_location('terrain_reference', ROOT/'tests/fixtures/terrain_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)

@unittest.skipUnless(BIN, 'set RUSTY_TILES_BIN for native terrain acceptance')
class NativeTerrainTests(unittest.TestCase):
    def source(self, root, *, crs=4326, bands=1, unit='m', scale=None, offset=None, nodata=False, gt=None):
        path=root/'dem.tif'
        ds=gdal.GetDriverByName('GTiff').Create(str(path),32,32,bands,gdal.GDT_Float32)
        if crs:
            srs=osr.SpatialReference();srs.ImportFromEPSG(crs);ds.SetProjection(srs.ExportToWkt())
        ds.SetGeoTransform(gt or [12,.01,0,42,0,-.01])
        values=100.125+np.add.outer(np.arange(32)*3.25,np.arange(32)*.125).astype(np.float32)
        if nodata:values[12:20,12:20]=-32768
        for i in range(bands):
            band=ds.GetRasterBand(i+1);band.WriteArray(values);band.SetUnitType(unit)
            if nodata:band.SetNoDataValue(-32768)
            if scale is not None:band.SetScale(scale)
            if offset is not None:band.SetOffset(offset)
        ds=None
        return path

    def call(self, source, out, *args, grid=17, zoom=9, height=10.25, fill=-999.125, error=0., env=None):
        result=subprocess.run([BIN,'--json','terrain','-i',str(source),'-o',str(out),
            '--maxZoom',str(zoom),'--grid',str(grid),'--heightOffset',str(height),'--fillHeight',str(fill),'--maxError',str(error),*args],
            capture_output=True,text=True,env=env or dict(os.environ,PATH=''))
        return result,json.loads(result.stdout)

    def test_all_grids_geographic_projected_and_constant_match_independent_oracle(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            for case_id,(grid,crs,fill) in enumerate([(17,4326,-999.125),(33,32633,-999.125),(65,4326,-999.125),(129,4326,-999.125),(17,4326,0.)]):
                with self.subTest(grid=grid,crs=crs):
                    source=self.source(root,crs=crs,nodata=True,gt=[500000,1000,0,4650000,0,-1000] if crs==32633 else None)
                    out=root/f'native-{case_id}';reference=root/f'oracle-{case_id}';reference.mkdir()
                    result,summary=self.call(source,out,'--progress','json',grid=grid,zoom=6,fill=fill)
                    self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(summary['ok'])
                    events=[json.loads(line) for line in result.stderr.splitlines()]
                    self.assertEqual(events[0]['phase'],'conversion');self.assertEqual(events[-1]['phase'],'conversion')
                    tile_events=[e for e in events if e['phase']=='terrain']
                    self.assertEqual(tile_events[0]['done'],0)
                    self.assertEqual(tile_events[-1]['done'],tile_events[-1]['total'])
                    oracle.run(types.SimpleNamespace(input=str(source),output=str(reference),max_zoom=6,grid=grid,
                        height_offset=10.25,fill_height=fill,max_tiles=100000))
                    self.assertEqual(json.loads((out/'layer.json').read_text()),json.loads((reference/'layer.json').read_text()))
                    native_report=json.loads((out/'conversion.json').read_text())
                    native_report.pop('simplification')
                    reference_report=json.loads((reference/'conversion.json').read_text())
                    native_report.pop('limitations');reference_report.pop('limitations')
                    self.assertEqual(native_report,reference_report)
                    paths=list(out.glob('*/*/*.terrain'))
                    self.assertEqual(len(paths),json.loads((out/'conversion.json').read_text())['tiles'])
                    for path in paths:
                        rel=path.relative_to(out)
                        header,attrs,tris,edges=decode_terrain(path.read_bytes())
                        expected,eattrs,etris,eedges=decode_terrain((reference/rel).read_bytes())
                        np.testing.assert_array_equal(attrs,eattrs);np.testing.assert_array_equal(tris,etris)
                        for edge,other in zip(edges,eedges):np.testing.assert_array_equal(edge,other)
                        self.assertEqual(header[3:5],expected[3:5])
                        np.testing.assert_allclose(header[:3]+header[5:9],expected[:3]+expected[5:9],rtol=1e-14,atol=1e-8)
                        self.assertTrue(np.isfinite(header).all())
                        if int(rel.parts[0])<2:self.assertGreater(np.linalg.norm(header[9:12]),1e12)
                        elif fill>=0:np.testing.assert_allclose(header[9:12],expected[9:12],rtol=1e-12,atol=1e-12)
                        xy=attrs[tris,:2]
                        area=(xy[:,1,0]-xy[:,0,0])*(xy[:,2,1]-xy[:,0,1])-(xy[:,1,1]-xy[:,0,1])*(xy[:,2,0]-xy[:,0,0])
                        self.assertTrue((area>0).all())
                        z,x,y=int(rel.parts[0]),int(rel.parts[1]),int(path.stem)
                        size=180/2**z
                        heights=header[3]+attrs[:,2]/32767*(header[4]-header[3])
                        xyz=oracle.ecef(-180+x*size+attrs[:,0]/32767*size,-90+y*size+attrs[:,1]/32767*size,heights)
                        self.assertLessEqual(np.linalg.norm(xyz-np.array(header[5:8]),axis=1).max(),header[8]+1e-7)
                        sidecar=rel.with_name(rel.name.replace('.terrain','.heights.json'))
                        self.assertEqual(json.loads((out/sidecar).read_text()),json.loads((reference/sidecar).read_text()))
                        for dx,dy,a,b in [(1,0,2,0),(0,1,3,1)]:
                            neighbor=out/str(z)/str(x+dx)/f'{y+dy}.terrain'
                            if neighbor.exists():
                                _,neighbor_attrs,_,neighbor_edges=decode_terrain(neighbor.read_bytes())
                                np.testing.assert_array_equal(attrs[edges[a],2],neighbor_attrs[neighbor_edges[b],2])
            source=self.source(root)
            ds=gdal.Open(str(source),gdal.GA_Update);ds.GetRasterBand(1).Fill(42);ds=None
            out=root/'constant';result,summary=self.call(source,out,zoom=0,height=0,fill=42)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(json.loads((out/'conversion.json').read_text())['heightRange'],[42,42])
            for path in out.glob('*/*/*.terrain'):
                header,attrs,*_=decode_terrain(path.read_bytes());self.assertEqual(header[3:5],(42,42))
                self.assertTrue((attrs[:,2]==0).all())

    def test_invalid_inputs_force_rollback_resource_limit_and_doctor(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);out=root/'output';out.mkdir();(out/'sentinel').write_text('keep')
            cases=[dict(crs=None),dict(bands=2),dict(unit='feet'),dict(scale=2),dict(offset=1),
                dict(gt=[-180,6,0,42,0,-.01]),dict(gt=[12,.01,0,92,0,-.01])]
            for case in cases:
                with self.subTest(case=case):
                    source=self.source(root,**case);result,report=self.call(source,out,'--force',zoom=0)
                    self.assertEqual(result.returncode,3,result.stderr);self.assertEqual(report['error']['code'],'data')
                    self.assertEqual((out/'sentinel').read_text(),'keep');self.assertEqual(set(root.iterdir()),{out,source})
            source=self.source(root)
            for args in [dict(zoom=25),dict(grid=18),dict(height=float('nan')),dict(fill=float('inf')),dict(fill=1e40),dict(zoom=24),dict(error=-1),dict(error=float('nan')),dict(error=float('inf'))]:
                result,report=self.call(source,out,'--force',**args)
                self.assertEqual(result.returncode,3,result.stderr);self.assertEqual((out/'sentinel').read_text(),'keep')
            result,report=self.call(source,out,zoom=0);self.assertEqual(result.returncode,5)
            result,report=self.call(source,out,'--force',zoom=0);self.assertEqual(result.returncode,0,result.stderr)
            self.assertFalse((out/'sentinel').exists());self.assertTrue((out/'layer.json').exists())
            result=subprocess.run([BIN,'doctor','--json','--command','terrain'],capture_output=True,text=True,env=dict(os.environ,PATH=''))
            self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(json.loads(result.stdout)['commands']['terrain']['ready'])

    def interpolated(self, full, reduced, west, south, size):
        header,base,_,_=full
        rheader,attrs,tris,_=reduced
        axis=np.unique(base[:,0]);grid=len(axis)
        # Restore the independent decoder's source grid from first-use ordering.
        ids=np.empty((grid,grid),dtype=int)
        for i,p in enumerate(base):ids[np.searchsorted(axis,p[1]),np.searchsorted(axis,p[0])]=i
        original_height=header[3]+base[:,2]/32767*(header[4]-header[3])
        height=rheader[3]+attrs[:,2]/32767*(rheader[4]-rheader[3])
        xyz=oracle.ecef(west+base[:,0]/32767*size,south+base[:,1]/32767*size,original_height)
        reduced_xyz=oracle.ecef(west+attrs[:,0]/32767*size,south+attrs[:,1]/32767*size,height)
        # Check cell centres and edge midpoints as well as original vertices.
        # Reference positions lie on the decoded source triangles, rather than
        # on a fresh ellipsoid surface that would introduce sampling error.
        dense=np.empty(grid*2-1);dense[::2]=axis;dense[1::2]=(axis[:-1]+axis[1:])/2
        xx,yy=np.meshgrid(dense,dense);coordinates=np.column_stack([xx.ravel(),yy.ravel()])
        cells=np.minimum(np.arange(len(dense))//2,grid-2)
        cx,cy=np.meshgrid(cells,cells)
        u=(xx-axis[cx])/(axis[cx+1]-axis[cx]);v=(yy-axis[cy])/(axis[cy+1]-axis[cy])
        lower=u+v<=1
        source_ids=np.stack([np.where(lower,ids[cy,cx],ids[cy,cx+1]),
            np.where(lower,ids[cy,cx+1],ids[cy+1,cx+1]),ids[cy+1,cx]],axis=-1)
        weights=np.stack([np.where(lower,1-u-v,1-v),np.where(lower,u,u+v-1),np.where(lower,v,1-u)],axis=-1)
        xyz=np.sum(xyz[source_ids]*weights[...,None],axis=-2).reshape(-1,3)
        original_height=np.sum(original_height[source_ids]*weights,axis=-1).ravel()
        axis=dense;ids=np.arange(len(dense)**2).reshape(len(dense),len(dense))
        output_height=np.full(len(coordinates),np.nan);output_xyz=np.full_like(xyz,np.nan)
        for triangle in tris:
            points=attrs[triangle,:2].astype(float)
            a,b,c=points
            area=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
            self.assertGreater(area,0)
            x0,x1=np.searchsorted(axis,[points[:,0].min(),points[:,0].max()],side='left')
            y0,y1=np.searchsorted(axis,[points[:,1].min(),points[:,1].max()],side='left')
            selected=ids[y0:y1+1,x0:x1+1].ravel()
            p=coordinates[selected]
            def cross(left,right,values):
                edge=right-left
                return edge[0]*(values[:,1]-left[1])-edge[1]*(values[:,0]-left[0])
            weights=np.array([cross(b,c,p),cross(c,a,p),cross(a,b,p)]).T/area
            valid=(weights>=-1e-14).all(axis=1);selected=selected[valid];weights=weights[valid]
            output_height[selected]=weights@height[triangle]
            output_xyz[selected]=weights@reduced_xyz[triangle]
        self.assertTrue(np.isfinite(output_height).all());self.assertTrue(np.isfinite(output_xyz).all())
        return float(np.abs(output_height-original_height).max()),float(np.linalg.norm(output_xyz-xyz,axis=1).max())

    def test_simplification_reduces_output_preserves_edges_and_bounds_added_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            for case,grid,limit in [('flat',65,1.),('hill',65,.25),('ridge-nodata',129,1.),('coarse-flat',17,1.)]:
                with self.subTest(case=case,grid=grid):
                    source=self.source(root,nodata=case=='ridge-nodata')
                    ds=gdal.Open(str(source),gdal.GA_Update)
                    yy,xx=np.mgrid[:32,:32]
                    values=np.full((32,32),123.5,dtype=np.float32)
                    if case=='hill':values+=(90*np.exp(-((xx-16)**2+(yy-16)**2)/15)).astype(np.float32)
                    if case=='ridge-nodata':
                        values+=(np.sin(xx*1.1)*25+np.cos(yy*1.3)*32).astype(np.float32)
                        values[:,15]+=200;values[12:20,12:20]=-32768
                    ds.GetRasterBand(1).WriteArray(values);ds=None
                    baseline=root/f'full-{case}';reduced=root/f'reduced-{case}'
                    result,_=self.call(source,baseline,grid=grid,zoom=9,error=0.,fill=0.)
                    self.assertEqual(result.returncode,0,result.stderr)
                    result,_=self.call(source,reduced,grid=grid,zoom=9,error=limit,fill=0.)
                    self.assertEqual(result.returncode,0,result.stderr)
                    full_report=json.loads((baseline/'conversion.json').read_text())
                    report=json.loads((reduced/'conversion.json').read_text());stats=report['simplification']
                    self.assertEqual(stats['maxErrorMetres'],limit)
                    self.assertLessEqual(stats['outputTriangles'],stats['inputTriangles'])
                    self.assertLessEqual(stats['outputVertices'],stats['inputVertices'])
                    if case in ('flat','hill'):
                        self.assertLess(stats['outputTriangles'],stats['inputTriangles'])
                        self.assertLess(stats['outputVertices'],stats['inputVertices'])
                    self.assertEqual(report['heightRange'],full_report['heightRange'])
                    self.assertEqual(json.loads((baseline/'layer.json').read_text()),json.loads((reduced/'layer.json').read_text()))
                    max_height=max_surface=0.
                    for path in baseline.glob('*/*/*.terrain'):
                        rel=path.relative_to(baseline);other=reduced/rel
                        full=decode_terrain(path.read_bytes());simple=decode_terrain(other.read_bytes())
                        self.assertLessEqual(other.stat().st_size,path.stat().st_size)
                        self.assertEqual(full[0],simple[0])
                        for edge,redge in zip(full[3],simple[3]):np.testing.assert_array_equal(full[1][edge],simple[1][redge])
                        sidecar=rel.with_name(rel.name.replace('.terrain','.heights.json'))
                        self.assertEqual((baseline/sidecar).read_bytes(),(reduced/sidecar).read_bytes())
                        z,x,y=int(rel.parts[0]),int(rel.parts[1]),int(path.stem);size=180/2**z
                        if len(full[1])==len(simple[1]):
                            np.testing.assert_array_equal(full[1],simple[1]);continue
                        height,surface=self.interpolated(full,simple,-180+x*size,-90+y*size,size)
                        self.assertLessEqual(height,limit+1e-7);self.assertLessEqual(surface,limit+1e-7)
                        max_height=max(max_height,height);max_surface=max(max_surface,surface)
                        xyz=oracle.ecef(-180+x*size+simple[1][:,0]/32767*size,-90+y*size+simple[1][:,1]/32767*size,
                            simple[0][3]+simple[1][:,2]/32767*(simple[0][4]-simple[0][3]))
                        self.assertLessEqual(np.linalg.norm(xyz-np.array(simple[0][5:8]),axis=1).max(),simple[0][8]+1e-7)
                    self.assertGreaterEqual(stats['maxAddedHeightErrorMetres'],max_height-1e-7)
                    self.assertLessEqual(stats['maxAddedHeightErrorMetres'],limit)
                    self.assertGreaterEqual(stats['maxAddedSurfaceErrorMetres'],max_surface-1e-7)
                    self.assertLessEqual(stats['maxAddedSurfaceErrorMetres'],limit)
                    # Kept edge samples stitch even when neighbouring tiles use different reductions.
                    for path in reduced.glob('*/*/*.terrain'):
                        z,x,y=int(path.parent.parent.name),int(path.parent.name),int(path.stem)
                        _,attrs,_,edges=decode_terrain(path.read_bytes())
                        for dx,dy,a,b in [(1,0,2,0),(0,1,3,1)]:
                            neighbor=reduced/str(z)/str(x+dx)/f'{y+dy}.terrain'
                            if neighbor.exists():
                                _,nattrs,_,nedges=decode_terrain(neighbor.read_bytes())
                                np.testing.assert_array_equal(attrs[edges[a],2],nattrs[nedges[b],2])
