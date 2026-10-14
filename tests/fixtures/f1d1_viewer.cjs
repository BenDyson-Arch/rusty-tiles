// Public Cesium tileVisible/getProperty observations; no private traversal edits.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath:process.env.CHROMIUM,headless:true,
    args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page=await browser.newPage({viewport:{width:1100,height:800}}), errors=[],failed=[],external=[];
    page.on('pageerror',error=>errors.push(String(error)));
    page.on('requestfailed',request=>failed.push(request.url()));
    await page.route('**/*',route=>{
      const url=new URL(route.request().url());
      if(url.hostname!=='127.0.0.1'){external.push(url.href);return route.abort();}
      return route.continue();
    });
    await page.goto(process.argv[2]); await page.waitForFunction(()=>window.viewer&&window.Cesium);
    const result=await page.evaluate(async()=>{
      const C=window.Cesium,viewer=window.viewer,scene=viewer.scene;
      const plan=await(await fetch('/plan.json')).json(),runs=[];
      scene.highDynamicRange=false;viewer.camera.frustum.near=.1;
      const vector=a=>new C.Cartesian3(...a);
      async function frame(){await new Promise(resolve=>{const remove=scene.postRender.addEventListener(()=>{remove();resolve();});scene.requestRender();});}
      for(const label of ['proxy_region','source_triangle']){
        const tiles=await C.Cesium3DTileset.fromUrl('/good/tileset.json',{maximumScreenSpaceError:16,skipLevelOfDetail:false,featureIdLabel:label});
        scene.primitives.add(tiles);
        let visible=[];
        const reset=scene.preRender.addEventListener(()=>{visible=[];});
        const observe=tiles.tileVisible.addEventListener(tile=>visible.push({url:tile.content.url,triangles:tile.content.trianglesLength}));
        for(const stage of ['coarse','fine']){
          const camera=plan.camera[stage];viewer.camera.setView({destination:vector(camera.destination),orientation:{direction:vector(camera.direction),up:vector(camera.up)}});
          let settled=false;
          for(let i=0;i<300;i++){
            await frame();
            const rootVisible=visible.some(tile=>tile.url.endsWith('/t/root.glb'));
            const correct=stage==='coarse'?rootVisible:visible.length>0&&!rootVisible;
            if(tiles.tilesLoaded&&correct){await frame();await frame();settled=true;break;}
            await new Promise(resolve=>setTimeout(resolve,10));
          }
          if(!settled)throw new Error('natural '+stage+' traversal did not settle '+JSON.stringify(visible));
          const samples=plan.targets.map(target=>{
            const screen=C.SceneTransforms.worldToWindowCoordinates(scene,vector(target.world));
            const picked=screen&&scene.pick(screen,1,1),owns=picked instanceof C.Cesium3DTileFeature&&picked.tileset===tiles;
            const expected=target[label],properties={};
            if(owns)for(const name of Object.keys(expected)){
              const value=picked.getProperty(name);
              properties[name]=ArrayBuffer.isView(value)?Array.from(value):value;
            }
            return {world:target.world,screen:screen?[screen.x,screen.y]:null,picked:owns,pickedType:picked?.constructor?.name,
              expected,properties,matches:owns&&JSON.stringify(properties)===JSON.stringify(expected)};
          });
          runs.push({label,stage,visible,maximumScreenSpaceError:tiles.maximumScreenSpaceError,
            modelMatrixIsIdentity:C.Matrix4.equals(tiles.modelMatrix,C.Matrix4.IDENTITY),samples});
        }
        reset();observe();scene.primitives.remove(tiles);
      }
      return {version:C.VERSION,camera:plan.camera,runs};
    });
    assert.equal(result.version,'1.146.0');
    for(const run of result.runs){
      assert.equal(run.modelMatrixIsIdentity,true);assert.equal(run.maximumScreenSpaceError,16);
      if(run.stage==='coarse'&&run.label==='proxy_region'||run.stage==='fine'&&run.label==='source_triangle')
        assert.ok(run.samples.every(sample=>sample.matches),'public '+run.stage+'/'+run.label+' properties '+JSON.stringify(run));
      else assert.ok(run.samples.every(sample=>!sample.matches),'missing label must not expose invented matching properties');
    }
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);assert.deepEqual(external,[]);
    console.log(JSON.stringify({...result,browserVersion:browser.version(),pageErrors:errors,failedRequests:failed,externalRequests:external},null,2));
  }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
