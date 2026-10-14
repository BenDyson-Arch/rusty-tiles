// Independent fixed-world cameras/targets. No Cesium geodetic/frame helpers and
// no presentation modelMatrix. Consumes each serialized tileset root unchanged.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
  const browser=await chromium.launch({executablePath:process.env.CHROMIUM,headless:true,args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page=await browser.newPage({viewport:{width:1000,height:800}}),errors=[],failed=[],external=[];
    page.on('pageerror',e=>errors.push(String(e)));page.on('requestfailed',r=>failed.push(r.url()));
    await page.route('**/*',route=>{const u=new URL(route.request().url());if(u.hostname!=='127.0.0.1'){external.push(u.href);return route.abort();}return route.continue();});
    await page.goto(process.argv[2]);await page.waitForFunction(()=>window.viewer&&window.Cesium);
    const result=await page.evaluate(async()=>{
      const C=window.Cesium,v=window.viewer,scene=v.scene;
      const plans=await (await fetch('/plan.json')).json(),runs=[];
      scene.highDynamicRange=false;
      const vector=a=>new C.Cartesian3(...a);
      async function frame(){await new Promise(resolve=>{const remove=scene.postRender.addEventListener(()=>{remove();resolve();});scene.requestRender();});}
      async function settle(predicate){for(let i=0;i<250;i++){await frame();if(predicate()){await frame();await frame();return;}await new Promise(r=>setTimeout(r,15));}throw new Error('placement render state did not settle');}
      function setCamera(plan,far=false){
        v.camera.lookAtTransform(C.Matrix4.IDENTITY);
        v.camera.frustum.near=plan.nearPlaneMetres;
        v.camera.setView({destination:vector(far?plan.farCamera.destination:plan.camera.destination),orientation:{direction:vector(plan.camera.direction),up:vector(plan.camera.up)}});
      }
      function snapshot(tiles,plan){
        // scene.pick performs a separate narrow-frustum render and changes
        // private traversal statistics. Capture ordinary-frame facts first.
        const ordinary={selectedLeaves:tiles._selectedTiles.length,renderCommands:tiles._statistics.numberOfCommands,
          triangles:tiles._selectedTiles.reduce((n,t)=>n+t.content.trianglesLength,0),rootScreenSpaceError:tiles.root._screenSpaceError};
        const samples=plan.targets.map(target=>{
          const screen=C.SceneTransforms.worldToWindowCoordinates(scene,vector(target.world));
          if(!screen)return {target:target.world,projected:null,picked:false};
          const picked=scene.pick(screen,1,1);
          const owns=!!picked&&(picked.primitive===tiles||picked.tileset===tiles||picked.content?.tile?.tileset===tiles);
          let depth=null,depthError=null;
          if(owns&&scene.pickPositionSupported){const p=scene.pickPosition(screen);if(p){depth=[p.x,p.y,p.z];depthError=C.Cartesian3.distance(p,vector(target.world));}}
          return {target:target.world,projected:[screen.x,screen.y],picked:owns,depthWorld:depth,depthErrorMetres:depthError};
        });
        return {...ordinary,modelMatrixIsIdentity:C.Matrix4.equals(tiles.modelMatrix,C.Matrix4.IDENTITY),samples};
      }
      for(const plan of plans){
        const cases={};
        for(const suffix of ['mesh','wrong-axis','wrong-order','wrong-height','missing-root','zero-error']){
          const tiles=await C.Cesium3DTileset.fromUrl('/'+plan.name+'/'+suffix+'/tileset.json',{maximumScreenSpaceError:8,skipLevelOfDetail:false});
          scene.primitives.add(tiles);setCamera(plan);
          if(suffix==='mesh')await settle(()=>tiles.tilesLoaded&&tiles._selectedTiles.length===3&&tiles._statistics.numberOfCommands>0);
          else {await settle(()=>tiles.tilesLoaded);for(let i=0;i<8;i++)await frame();}
          cases[suffix]=snapshot(tiles,plan);
          if(suffix==='mesh'){
            setCamera(plan,true);await settle(()=>tiles._selectedTiles.length===0&&tiles.root._screenSpaceError<8);
            cases.far=snapshot(tiles,plan);setCamera(plan);
            await settle(()=>tiles.tilesLoaded&&tiles._selectedTiles.length===3&&tiles._statistics.numberOfCommands>0);
            cases.returnedNear=snapshot(tiles,plan);
          }
          scene.primitives.remove(tiles);
        }
        runs.push({name:plan.name,placement:plan.placement,independentCamera:plan.camera,independentTargets:plan.targets,
                   scale:plan.scale,nearPlaneMetres:plan.nearPlaneMetres,cases});
      }
      return {version:C.VERSION,viewport:[1000,800],maximumScreenSpaceError:8,pickPositionSupported:scene.pickPositionSupported,runs};
    });
    assert.equal(result.version,'1.146.0');
    for(const run of result.runs){
      const good=run.cases.mesh;
      assert.equal(good.modelMatrixIsIdentity,true,'presentation transform must remain identity');
      assert.equal(good.selectedLeaves,3);assert.equal(good.triangles,3);assert.ok(good.renderCommands>0);
      assert.ok(good.samples.every(p=>p.picked),'all independent world targets must pick candidate tiles');
      assert.equal(run.cases.far.selectedLeaves,0);assert.ok(run.cases.returnedNear.samples.every(p=>p.picked));
      for(const control of ['wrong-axis','wrong-order','wrong-height','missing-root']){
        assert.ok(run.cases[control].samples.some(p=>!p.picked),run.name+' '+control+' must miss independent target');
      }
      assert.equal(run.cases['zero-error'].selectedLeaves,0);assert.ok(run.cases['zero-error'].samples.every(p=>!p.picked));
      // Depth includes pixel-centre/raster precision, so it is measured rather
      // than used as the nanometre offline arithmetic oracle.
    }
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);assert.deepEqual(external,[]);
    console.log(JSON.stringify({...result,browserVersion:browser.version(),pageErrors:errors,failedRequests:failed,externalRequests:external},null,2));
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
