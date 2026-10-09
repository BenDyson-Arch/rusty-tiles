// Optional acceptance against native terrain served by the actual preview.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM || '/usr/bin/chromium',headless:true,
    args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport:{width:1100,height:800}});
    const errors=[]; page.on('pageerror',error=>errors.push(String(error)));
    await page.goto(process.argv[2] || 'http://127.0.0.1:9272');
    async function inspect() {
      await page.waitForFunction(()=>window.terrain || window.failures?.length,null,{timeout:30000});
      return page.evaluate(async()=>{
        const C=Cesium,viewer=window.viewer,provider=window.terrain;
        if(!provider)throw new Error(window.failures.join('\n'));
        const [covered,hole]=await C.sampleTerrain(provider,9,[
          C.Cartographic.fromDegrees(12.05,41.95),C.Cartographic.fromDegrees(12.16,41.84)],true);
        const [outside]=await C.sampleTerrain(provider,9,[C.Cartographic.fromDegrees(12.8,41.8)]);
        const root=await provider.requestTileGeometry(0,0,0);
        const manifest=await (await fetch('/terrain/layer.json')).json();
        const conversion=await(await fetch('/terrain/conversion.json')).json();
        const scheme=provider.tilingScheme;
        const tile=scheme.positionToTileXY(covered,9);
        const tmsY=scheme.getNumberOfYTilesAtLevel(9)-tile.y-1;
        const sidecar=await(await fetch(`/terrain/9/${tile.x}/${tmsY}.heights.json`)).json();
        viewer.camera.setView({destination:C.Rectangle.fromDegrees(...manifest.bounds)});
        await new Promise(resolve=>setTimeout(resolve,500));
        for(let i=0;i<100 && !viewer.scene.globe.tilesLoaded;i++)await new Promise(resolve=>setTimeout(resolve,50));
        viewer.scene.render();
        return {version:C.VERSION,covered:covered.height,hole:hole.height,outside:outside.height ?? null,
          rootVertices:root._uValues?.length,rootAvailable:provider.getTileDataAvailable(0,0,0),
          rendered:viewer.scene.globe.tilesLoaded,sidecarNulls:sidecar.heights.filter(h=>h===null).length,
          sidecarValues:sidecar.heights.filter(h=>h!==null),step:conversion.heightQuantizationStep,
          failures:window.failures.slice()};
      });
    }
    const first=await inspect();
    const cdp=await page.context().newCDPSession(page);await cdp.send('Network.setCacheDisabled',{cacheDisabled:true});
    await page.reload({waitUntil:'load'});const refreshed=await inspect();
    console.log(JSON.stringify({first,refreshed,browserErrors:errors},null,2));
    assert.equal(errors.length,0);
    for(const result of [first,refreshed]) {
      assert.equal(result.version,'1.146.0');assert.equal(result.failures.length,0);
      assert.ok(Math.abs(result.covered-133.75)<=result.step/2+1e-6,`Covered height: ${result.covered}`);
      assert.ok(Math.abs(result.hole+999.125)<1e-6,`NoData height: ${result.hole}`);
      assert.ok(result.rootVertices>0);assert.equal(result.rootAvailable,true);assert.equal(result.rendered,true);
      assert.ok(result.sidecarNulls>0);assert.ok(result.sidecarValues.length>0);
      assert.ok(result.sidecarValues.every(h=>h===133.75));
    }
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
