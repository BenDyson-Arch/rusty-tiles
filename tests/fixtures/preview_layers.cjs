// Native-preview five-layer acceptance. Existing probes audit detailed geometry/picking.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
  const browser=await chromium.launch({executablePath:process.env.CHROMIUM || '/usr/bin/chromium',headless:true,
    args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page=await browser.newPage({viewport:{width:1100,height:800}});
    const errors=[],external=[];
    page.on('pageerror',e=>errors.push(String(e)));
    await page.route('**/*',route=>{
      const url=new URL(route.request().url());
      if(!['127.0.0.1','localhost'].includes(url.hostname)){external.push(url.href);return route.abort();}
      return route.continue();
    });
    await page.goto(process.argv[2]);
    const inspect=async()=>{
      await page.waitForFunction(()=>window.tiles && window.annotations && window.pointCloud && window.orthophoto && window.terrain,null,{timeout:30000});
      const toggles=page.locator('#layers input');assert.equal(await toggles.count(),4);
      for(let i=0;i<4;i++){
        await toggles.nth(i).uncheck();
        assert.equal(await page.evaluate(i=>[window.orthophoto,window.tiles,window.annotations,window.pointCloud][i].show,i),false);
        await toggles.nth(i).check();
      }
      await page.getByRole('button',{name:'3D mesh extent',exact:true}).click();
      return page.evaluate(async()=>{
        const C=Cesium,viewer=window.viewer,mesh=window.tiles;
        window.annotations.show=false;window.pointCloud.show=false;
        viewer.scene.globe.show=false;
        for(let i=0;i<100 && !mesh.tilesLoaded;i++)await new Promise(r=>setTimeout(r,50));
        await new Promise(r=>setTimeout(r,500));
        let picked;
        for(let i=0;i<100 && !picked;i++) {
          viewer.scene.render();
          const center=C.SceneTransforms.worldToWindowCoordinates(viewer.scene,mesh.boundingSphere.center);
          picked=center && viewer.scene.pick(center,3,3);
          if(!picked)await new Promise(r=>setTimeout(r,50));
        }
        // Real imagery data and IIFE worker/runtime routes must decode too.
        const image=await window.orthophoto.imageryProvider.requestImage(546,380,10);
        return {version:C.VERSION,meshLoaded:mesh.tilesLoaded,meshPicked:!!picked,
          imagerySize:[image.width,image.height],failures:window.failures};
      });
    };
    const first=await inspect();
    const cdp=await page.context().newCDPSession(page);await cdp.send('Network.setCacheDisabled',{cacheDisabled:true});
    await page.reload({waitUntil:'load'});const refreshed=await inspect();
    console.log(JSON.stringify({first,refreshed,browserErrors:errors,externalRequests:external},null,2));
    for(const result of [first,refreshed]){
      assert.equal(result.version,'1.143.0');assert.equal(result.meshLoaded,true);assert.equal(result.meshPicked,true);
      assert.deepEqual(result.imagerySize,[256,256]);assert.deepEqual(result.failures,[]);
    }
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
