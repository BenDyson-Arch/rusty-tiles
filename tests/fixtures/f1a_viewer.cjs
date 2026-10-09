// Actual render-frame selection for F1a's empty-root omission/selection metric.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
  const url=process.argv[2],expectedLeaves=Number(process.argv[3]);
  const browser=await chromium.launch({executablePath:process.env.CHROMIUM||'/usr/bin/chromium',headless:true,
    args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page=await browser.newPage({viewport:{width:1000,height:800}});
    const errors=[],failed=[],external=[];
    page.on('pageerror',e=>errors.push(String(e)));
    page.on('requestfailed',r=>failed.push(r.url()));
    await page.route('**/*',route=>{
      const u=new URL(route.request().url());
      if(u.hostname!=='127.0.0.1'){external.push(u.href);return route.abort();}
      return route.continue();
    });
    await page.goto(url);
    await page.waitForFunction(()=>window.viewer&&window.Cesium);
    const result=await page.evaluate(async({expectedLeaves})=>{
      const C=Cesium,v=window.viewer,scene=v.scene;
      const matrix=C.Transforms.eastNorthUpToFixedFrame(C.Cartesian3.fromDegrees(0,0,0));
      const tiles=await C.Cesium3DTileset.fromUrl('/mesh/tileset.json',{maximumScreenSpaceError:8,skipLevelOfDetail:false});
      tiles.modelMatrix=matrix;scene.primitives.add(tiles);
      const center=tiles.boundingSphere.center;
      const radius=tiles.boundingSphere.radius;
      const frame=()=>new Promise(resolve=>{const remove=scene.postRender.addEventListener(()=>{remove();resolve();});scene.requestRender();});
      function camera(distance){v.camera.lookAt(center,new C.HeadingPitchRange(0,-Math.PI/3,distance));}
      async function settled(predicate){
        for(let i=0;i<300;i++){
          await frame();
          if(predicate()){await frame();await frame();return;}
          await new Promise(r=>setTimeout(r,20));
        }
        throw new Error('render state did not settle');
      }
      function snapshot(){return {selectedLeaves:tiles._selectedTiles.length,renderCommands:tiles._statistics.numberOfCommands,
        rootScreenSpaceError:tiles.root._screenSpaceError,tilesLoaded:tiles.tilesLoaded,
        triangles:tiles._selectedTiles.reduce((n,t)=>n+t.content.trianglesLength,0)};}
      camera(4*radius);
      await settled(()=>tiles.tilesLoaded&&tiles._selectedTiles.length===expectedLeaves&&tiles._statistics.numberOfCommands>0);
      const near=snapshot();
      // A real triangle-centre pick proves commands reach visible rendered content.
      const testPoint=C.Matrix4.multiplyByPoint(matrix,new C.Cartesian3(1/3,-1/3,1/3),new C.Cartesian3());
      const windowPoint=C.SceneTransforms.worldToWindowCoordinates(scene,testPoint);
      near.picked=!!(windowPoint&&scene.pick(windowPoint,3,3));
      camera(10000*radius);
      await settled(()=>tiles._selectedTiles.length===0&&tiles.root._screenSpaceError<8);
      const far=snapshot();
      camera(4*radius);
      await settled(()=>tiles._selectedTiles.length===expectedLeaves&&tiles._statistics.numberOfCommands>0);
      const returnedNear=snapshot();
      scene.primitives.remove(tiles);
      const zero=await C.Cesium3DTileset.fromUrl('/zero/tileset.json',{maximumScreenSpaceError:8,skipLevelOfDetail:false});
      zero.modelMatrix=matrix;scene.primitives.add(zero);
      camera(4*radius);await frame();await frame();await frame();
      const negative={selectedLeaves:zero._selectedTiles.length,rootScreenSpaceError:zero.root._screenSpaceError};
      return {version:C.VERSION,near,far,returnedNear,zeroRootControl:negative,viewport:[1000,800],maximumScreenSpaceError:8,
        headingRadians:0,pitchRadians:-Math.PI/3,nearDistanceMetres:4*radius,farDistanceMetres:10000*radius,radiusMetres:radius};
    },{expectedLeaves});
    assert.equal(result.version,'1.146.0');
    assert.equal(result.near.selectedLeaves,expectedLeaves);assert.equal(result.near.triangles,12);assert.equal(result.near.picked,true);
    assert.equal(result.far.selectedLeaves,0);assert.equal(result.returnedNear.selectedLeaves,expectedLeaves);
    assert.equal(result.zeroRootControl.selectedLeaves,0);assert.equal(result.zeroRootControl.rootScreenSpaceError,0);
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);assert.deepEqual(external,[]);
    console.log(JSON.stringify({...result,browserVersion:browser.version(),pageErrors:errors,failedRequests:failed,externalRequests:external},null,2));
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
