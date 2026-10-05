// Browser acceptance for the styling and picking examples in docs/VECTOR.md.
const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
    headless:true,args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page=await browser.newPage({viewport:{width:1100,height:800}});
    const errors=[];page.on('pageerror',error=>errors.push(String(error)));
    await page.goto(process.argv[2] || 'http://127.0.0.1:9257');
    await page.waitForFunction(()=>window.annotations || window.failures?.length,{timeout:30000});
    const report=await page.evaluate(async()=>{
      const C=Cesium,v=window.viewer,wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
      v.scene.globe.show=false;v.scene.skyAtmosphere.show=false;v.scene.skyBox.show=false;
      if(window.annotations)v.scene.primitives.remove(window.annotations);
      const cases=await(await fetch('/annotations/cases.json')).json(),results=[];
      function pick(position){
        const p=C.SceneTransforms.worldToWindowCoordinates(v.scene,C.Cartesian3.fromDegrees(...position));
        return p && v.scene.pick(p,1,1);
      }
      function boundaryFeature(entry){
        if(!entry.boundary)return null;
        const p=C.SceneTransforms.worldToWindowCoordinates(v.scene,C.Cartesian3.fromDegrees(...entry.boundary));
        const center=C.SceneTransforms.worldToWindowCoordinates(v.scene,C.Cartesian3.fromDegrees(...entry.hole));
        const dx=p.x-center.x,dy=p.y-center.y,length=Math.hypot(dx,dy);
        p.x+=5*dx/length;p.y+=5*dy/length;
        return v.scene.pick(p,1,1);
      }
      function values(feature){
        if(!feature?.getProperty)return null;
        return Object.fromEntries(feature.getPropertyIds().map(key=>{
          const value=feature.getProperty(key);return [key,{type:typeof value,value:String(value)}];
        }));
      }
      for(const entry of cases){
        window.failures=[];
        const tiles=await C.Cesium3DTileset.fromUrl(`/annotations/${entry.name}/tileset.json`,{
          maximumScreenSpaceError:1e-7,backFaceCulling:false,dynamicScreenSpaceError:false,foveatedScreenSpaceError:false});
        v.scene.primitives.add(tiles);
        tiles.tileFailed.addEventListener(error=>window.failures.push(String(error.message || error)));
        v.camera.viewBoundingSphere(tiles.boundingSphere,new C.HeadingPitchRange(0,-Math.PI/2,Math.max(tiles.boundingSphere.radius*3,30)));
        v.camera.lookAtTransform(C.Matrix4.IDENTITY);
        tiles.style=new C.Cesium3DTileStyle({color:{conditions:[
          ["${category} === 'survey'","color('cyan')"],["true","color('orange')"]]},
          show:true,lineWidth:12,pointSize:12});
        await wait(100);
        for(let i=0;i<150 && !tiles.tilesLoaded;i++)await wait(50);
        await wait(400);
        const active=pick(entry.sample),missing=pick(entry.missing);
        const activeValues=values(active),missingValues=values(missing),boundary=values(boundaryFeature(entry));
        const colors={active:active?.color?.toCssColorString(),missing:missing?.color?.toCssColorString()};
        tiles.style=new C.Cesium3DTileStyle({color:"color('cyan')",show:"${status} === 'active'",lineWidth:12,pointSize:12});
        await wait(400);
        results.push({case:entry.name,active:activeValues,missing:missingValues,boundary,colors,
          activeAfterFilter:!!pick(entry.sample),missingAfterFilter:!!pick(entry.missing),failures:window.failures.slice()});
        v.scene.primitives.remove(tiles);
      }
      return {version:C.VERSION,cases:results};
    });
    console.log(JSON.stringify({...report,browserErrors:errors},null,2));
    const passed=!errors.length && report.cases.length===5 && report.cases.every(entry=>
      !entry.failures.length && entry.activeAfterFilter && !entry.missingAfterFilter &&
      entry.active?._source_id && entry.active?._source_layer &&
      (entry.case!=='fragmented' || (entry.boundary?._source_id?.value===entry.active._source_id.value &&
        entry.boundary?.category?.value==='survey')) &&
      entry.colors.active==='rgb(0,255,255)' && entry.colors.missing==='rgb(255,165,0)' &&
      entry.active.optional_integer.value==='1152921504606846979' &&
      entry.missing?.optional_integer?.type==='bigint' &&
      entry.missing?.optional_integer?.value==='-9007199254740992' &&
      ['optional_text','optional_number','name'].every(key=>entry.missing?.[key]?.type==='undefined'));
    if(!passed){console.error('Metadata styling/picking/noData acceptance failed.');process.exitCode=1;}
  }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
