// Standard Three0.180.0 PBR pipeline; independent source and extracted leaf GLBs.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
  const browser=await chromium.launch({executablePath:process.env.CHROMIUM,headless:true,args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try{
    const page=await browser.newPage({viewport:{width:1200,height:900}}),errors=[],failed=[],external=[];
    page.on('pageerror',e=>errors.push(String(e)));page.on('requestfailed',r=>failed.push(r.url()));
    await page.route('**/*',route=>{const u=new URL(route.request().url());if(u.protocol!=='blob:'&&u.hostname!=='127.0.0.1'){external.push(u.href);return route.abort();}return route.continue();});
    await page.goto(process.argv[2]);await page.waitForFunction(()=>window.THREE&&window.GLTFLoader);
    const result=await page.evaluate(async expectedLeaves=>{
      const T=window.THREE,renderer=new T.WebGLRenderer({antialias:false,preserveDrawingBuffer:true});
      renderer.setSize(1200,900);renderer.setPixelRatio(1);renderer.outputColorSpace=T.SRGBColorSpace;renderer.toneMapping=T.NoToneMapping;
      document.body.appendChild(renderer.domElement);
      const scene=new T.Scene();scene.background=new T.Color(0);
      const camera=new T.OrthographicCamera(-5.5,5.5,4.125,-4.125,.1,50);camera.position.set(0,0,12);camera.lookAt(0,0,0);
      scene.add(new T.AmbientLight(0xffffff,1.2));
      const direct=new T.DirectionalLight(0xffffff,1);direct.position.set(-.35,1.25,1);scene.add(direct);
      const loader=new window.GLTFLoader();
      async function display(group){
        scene.add(group);scene.updateMatrixWorld(true);await renderer.compileAsync(scene,camera);
        renderer.render(scene,camera);await new Promise(r=>requestAnimationFrame(r));renderer.render(scene,camera);
      }
      function samples(){
        return Array.from({length:6},(_,i)=>{
          const left=-3.4+2.4*(i%3),bottom=-2.2+2.4*Math.floor(i/3),pixels=[];
          for(let row=0;row<2;row++)for(let col=0;col<3;col++){
            const point=[1.25*(left+2*(col+.5)/3),1.25*(bottom+2-2*(row+.5)/2),0];
            const ndc=new T.Vector3(...point).project(camera);const x=Math.floor((ndc.x*.5+.5)*1200),y=Math.floor((ndc.y*.5+.5)*900);
            if(x<0||x>=1200||y<0||y>=900)throw new Error('analytical sample offscreen');
            const gl=renderer.getContext(),out=new Uint8Array(4);gl.readPixels(x,y,1,1,gl.RGBA,gl.UNSIGNED_BYTE,out);
            if(gl.getError()!==gl.NO_ERROR)throw new Error('WebGL sample error');
            pixels.push({gltfWorld:point,framebuffer:[x,y],rgba:Array.from(out)});
          }
          return pixels;
        });
      }
      const source=(await loader.loadAsync('/reference/source.glb')).scene;
      await display(source);const reference=samples(),referenceCommands=renderer.info.render.calls;scene.remove(source);
      const runs={};
      for(const prefix of ['mesh','wrong-uv','wrong-slot','wrong-w','no-mr','no-normal','no-occlusion','no-emissive','wrong-color','wrong-scale','wrong-strength']){
        const tileset=await(await fetch('/'+prefix+'/tileset.json')).json();
        const leaves=tileset.root.children.map(c=>c.content.uri);
        if(leaves.length!==expectedLeaves)throw new Error('independent leaf inventory mismatch');
        const group=new T.Group();for(const leaf of leaves)group.add((await loader.loadAsync('/'+prefix+'/'+leaf)).scene);
        await display(group);runs[prefix]={panels:samples(),loadedLeaves:leaves.length,renderCommands:renderer.info.render.calls};scene.remove(group);
      }
      return {version:T.REVISION,reference,referenceCommands,runs,viewport:[1200,900],orthographicBounds:[-5.5,5.5,4.125,-4.125],cameraGltf:[0,0,12],
        directionalLightPosition:[-.35,1.25,1],directionalIntensity:1,ambientIntensity:1.2,
        shading:'Original Three MeshStandardMaterial/GLTFLoader PBR for independently authored source and each extracted leaf; no shader or material overrides',
        scope:'Six nearest/clamp panels and positive uniform scale, one pinned PBR renderer; extracted leaf material appearance, separate Cesium traversal evidence required'};
    },Number(process.argv[3]));
    assert.equal(result.version,'180');
    function delta(panels){return Math.max(...panels.flatMap((p,i)=>p.flatMap((s,j)=>s.rgba.slice(0,3).map((x,c)=>Math.abs(x-result.reference[i][j].rgba[c])))));}
    const positiveDelta=delta(result.runs.mesh.panels);assert(positiveDelta<=12,'lit source/output maximum RGB mismatch '+positiveDelta);
    const controls={};for(const [name,run] of Object.entries(result.runs))if(name!=='mesh'){controls[name]=delta(run.panels);assert(controls[name]>12,'appearance checker insensitive to '+name+': '+controls[name]);}
    assert(result.referenceCommands>0&&result.runs.mesh.renderCommands>0,'real lit draw commands required');
    assert(result.reference.every(panel=>panel.some(p=>p.rgba.slice(0,3).some(x=>x>8))),'all source panels visibly render');
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);assert.deepEqual(external,[]);
    console.log(JSON.stringify({...result,positiveMaximumRGBDelta:positiveDelta,sensitiveMaximumRGBDeltas:controls,browserVersion:browser.version(),pageErrors:errors,failedRequests:failed,externalRequests:external},null,2));
  }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
