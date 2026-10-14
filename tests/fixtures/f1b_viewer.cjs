// Pinned real-viewer texel/UV/OPAQUE/MASK acceptance. No source extensions needed.
// Display-only unlit shading isolates material baseColor/alpha from lighting.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
  const url=process.argv[2];
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
    const result=await page.evaluate(async()=>{
      const C=Cesium,v=window.viewer,scene=v.scene;
      scene.backgroundColor=C.Color.BLACK;
      scene.highDynamicRange=false;
      scene.postProcessStages.fxaa.enabled=false;
      const matrix=C.Transforms.eastNorthUpToFixedFrame(C.Cartesian3.fromDegrees(0,0,0));
      const toWorld=p=>C.Matrix4.multiplyByPoint(matrix,new C.Cartesian3(...p),new C.Cartesian3());
      const direction=C.Matrix4.multiplyByPointAsVector(matrix,new C.Cartesian3(0,1,0),new C.Cartesian3());
      const up=C.Matrix4.multiplyByPointAsVector(matrix,new C.Cartesian3(0,0,1),new C.Cartesian3());
      v.camera.setView({destination:toWorld([0,-7,0]),orientation:{direction,up}});
      const frame=()=>new Promise(resolve=>{const remove=scene.postRender.addEventListener(()=>{remove();resolve();});scene.requestRender();});
      async function load(prefix){
        const tiles=await C.Cesium3DTileset.fromUrl('/'+prefix+'/tileset.json',{maximumScreenSpaceError:8,
          customShader:new C.CustomShader({lightingModel:C.LightingModel.UNLIT})});
        tiles.modelMatrix=matrix;scene.primitives.add(tiles);
        for(let i=0;i<300;i++){
          await frame();
          if(tiles.tilesLoaded&&tiles._selectedTiles.length===2&&tiles._statistics.numberOfCommands>0){
            await frame();await frame();return tiles;
          }
          await new Promise(r=>setTimeout(r,20));
        }
        throw new Error('appearance tiles failed to settle: '+prefix);
      }
      function pixel(point){
        const windowPoint=C.SceneTransforms.worldToWindowCoordinates(scene,toWorld(point));
        assertPoint(windowPoint);
        const gl=scene.context._gl,pixel=new Uint8Array(4);
        gl.readPixels(Math.floor(windowPoint.x),gl.drawingBufferHeight-1-Math.floor(windowPoint.y),1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
        if(gl.getError()!==gl.NO_ERROR)throw new Error('WebGL readPixels failed');
        return {worldLocal:point,window:[windowPoint.x,windowPoint.y],rgba:Array.from(pixel)};
      }
      function assertPoint(p){if(!p||p.x<0||p.x>=1000||p.y<0||p.y>=800)throw new Error('analytic sample outside viewport');}
      function samples(){
        const panels=[];
        for(const left of [-2.2,0.2]){
          const pixels=[];
          for(let row=0;row<2;row++)for(let col=0;col<3;col++)pixels.push(pixel([left+2*(col+0.5)/3,0,1-2*(row+0.5)/2]));
          panels.push(pixels);
        }
        return panels;
      }
      const positiveTiles=await load('mesh');
      const positive={selectedLeaves:positiveTiles._selectedTiles.length,renderCommands:positiveTiles._statistics.numberOfCommands,panels:samples()};
      scene.primitives.remove(positiveTiles);
      const wrongUV=await load('wrong-uv');const uvControl=samples();scene.primitives.remove(wrongUV);
      const wrongAlpha=await load('wrong-alpha');const alphaControl=samples();scene.primitives.remove(wrongAlpha);
      return {version:C.VERSION,positive,wrongUVControl:uvControl,wrongAlphaControl:alphaControl,
        viewport:[1000,800],cameraLocal:[0,-7,0],maximumScreenSpaceError:8,shading:'Cesium CustomShader LightingModel.UNLIT; original baseColor texture/factors/sampler/alpha are retained',
        limits:'Fixed face-on nearest/clamp analytical sample points; no universal viewer, lighting, color-management or arbitrary sampler rendering claim.'};
    });
    assert.equal(result.version,'1.146.0');
    const expected=[[255,0,0],[0,255,0],[0,0,255],[255,255,0],[255,0,255],[0,255,255]];
    function matches(panels){
      return panels.every((panel,index)=>panel.every((sample,i)=>{
        const wanted=index===1&&(i===2||i===3)?[0,0,0]:expected[i];
        return wanted.every((v,c)=>Math.abs(sample.rgba[c]-v)<=12);
      }));
    }
    assert.ok(matches(result.positive.panels),'analytical rendered texels/alpha must match: '+JSON.stringify(result.positive.panels));
    assert.equal(matches(result.wrongUVControl),false,'UV negative control must fail appearance');
    assert.equal(matches(result.wrongAlphaControl),false,'alpha-mode negative control must fail appearance');
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);assert.deepEqual(external,[]);
    console.log(JSON.stringify({...result,browserVersion:browser.version(),pageErrors:errors,failedRequests:failed,externalRequests:external},null,2));
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
