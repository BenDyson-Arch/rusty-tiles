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
      const toggles=page.locator('#layers input');assert.equal(await toggles.count(),5);
      for(let i=0;i<5;i++){
        await toggles.nth(i).uncheck();
        assert.equal(await page.evaluate(i=>[window.terrain,window.orthophoto,window.tiles,window.annotations,window.pointCloud][i].show,i),false);
        await toggles.nth(i).check();
      }
      await page.getByRole('button',{name:'3D mesh extent',exact:true}).click();
      return page.evaluate(async()=>{
        const C=Cesium,viewer=window.viewer,mesh=window.tiles;
        window.annotations.show=false;window.pointCloud.show=false;
        viewer.scene.globe.show=false;
        // Software rendering (CI) can be slow: one run used up 600 renders without the mesh
        // reporting loaded. Render until tilesLoaded, waking early on the allTilesLoaded event,
        // for at least 120 s and 600 renders; if it never settles, the result carries the
        // tileset's request and processing counts.
        const tileFailures=[];
        mesh.tileFailed.addEventListener(e=>tileFailures.push(`${e.url}: ${e.message}`));
        const settle=async()=>{
          const start=performance.now();
          let wake=()=>{},renders=0;
          const remove=mesh.allTilesLoaded.addEventListener(()=>wake());
          try {
            while(!mesh.tilesLoaded && (performance.now()-start<120000 || renders<600)){
              viewer.scene.render();renders++;
              await new Promise(r=>{wake=r;setTimeout(r,50);});
            }
          } finally {remove();}
          return {ms:Math.round(performance.now()-start),renders};
        };
        const diagnostics=label=>{
          const s=mesh.statistics || mesh._statistics || {};
          return {label,tilesLoaded:mesh.tilesLoaded,pendingRequests:s.numberOfPendingRequests,
            processing:s.numberOfTilesProcessing,attemptedRequests:s.numberOfAttemptedRequests,
            selected:s.selected,visited:s.visited,contentReady:s.numberOfTilesWithContentReady,
            loadedTotal:s.numberOfLoadedTilesTotal,tileFailures:tileFailures.slice()};
        };
        const waits={load:await settle()};
        const afterLoad=diagnostics('after load');
        await new Promise(r=>setTimeout(r,500));
        let picked;
        for(let i=0;i<100 && !picked;i++) {
          viewer.scene.render();
          const center=C.SceneTransforms.worldToWindowCoordinates(viewer.scene,mesh.boundingSphere.center);
          picked=center && viewer.scene.pick(center,3,3);
          if(!picked)await new Promise(r=>setTimeout(r,50));
        }
        // Picking can render the frame that finishes the tile; settle tilesLoaded after it.
        if(picked)waits.pick=await settle();
        // Real imagery data and IIFE worker/runtime routes must decode too.
        const image=await window.orthophoto.imageryProvider.requestImage(546,380,10);
        const meshLoaded=mesh.tilesLoaded;
        return {version:C.VERSION,meshLoaded,meshPicked:!!picked,
          imagerySize:[image.width,image.height],failures:window.failures,waits,
          ...(meshLoaded && picked ? {} : {diagnostics:[afterLoad,diagnostics('at the end')]})};
      });
    };
    const first=await inspect();
    const cdp=await page.context().newCDPSession(page);await cdp.send('Network.setCacheDisabled',{cacheDisabled:true});
    await page.reload({waitUntil:'load'});const refreshed=await inspect();
    console.log(JSON.stringify({first,refreshed,browserErrors:errors,externalRequests:external},null,2));
    for(const result of [first,refreshed]){
      const state=JSON.stringify(result.diagnostics);
      assert.equal(result.version,'1.146.0');
      assert.equal(result.meshLoaded,true,`the mesh never reported tilesLoaded: ${state}`);
      assert.equal(result.meshPicked,true,`the mesh could not be picked: ${state}`);
      assert.deepEqual(result.imagerySize,[256,256]);assert.deepEqual(result.failures,[]);
    }
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
