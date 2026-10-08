// Load deep native implicit fixtures, cross subtree/external-root boundaries,
// check every instantiated level, and pick rendered mesh/point content.
// Usage: NODE_PATH=... node implicit_levels.cjs PREVIEW_URL
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
    args:['--no-sandbox','--enable-unsafe-swiftshader','--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport:{width:1100,height:800}});
    const errors=[];page.on('pageerror',error=>errors.push(String(error)));
    await page.goto(process.argv[2]);
    await page.waitForFunction(()=>window.tiles && window.pointCloud && window.annotations);
    const results=await page.evaluate(async()=>{
      const C=Cesium,v=window.viewer,wait=ms=>new Promise(r=>setTimeout(r,ms));
      v.scene.globe.show=false;v.scene.skyAtmosphere.show=false;v.scene.skyBox.show=false;
      v.scene.requestRenderMode=false;
      window.annotations.show=false;window.pointCloud.show=false;window.tiles.show=false;
      const outputs=[];
      function level(uri,octree) {
        if(!uri)return undefined;
        const name=uri.split('/').pop();
        if(name.startsWith('implicit-tileset-')) {
          const parts=name.slice(17).replace('.json','').split('-').map(Number);
          return parts[0]+parts[octree?4:3];
        }
        if(!name.endsWith('.glb')&&!name.endsWith('.b3dm'))return undefined;
        const parts=name.replace(/\.(glb|b3dm)$/,'').split('-').map(Number);
        return parts[0]+(parts.length===(octree?9:7)?parts[octree?4:3]:0);
      }
      for(const [name,tiles,octree] of [['mesh',window.tiles,true],['point-cloud',window.pointCloud,true]]) {
        tiles.show=true;tiles.maximumScreenSpaceError=8;
        function frame(multiplier){v.camera.viewBoundingSphere(tiles.boundingSphere,
          new C.HeadingPitchRange(0,-Math.PI/2,tiles.boundingSphere.radius*multiplier));v.camera.lookAtTransform(C.Matrix4.IDENTITY);}
        async function settle(){await wait(250);for(let i=0;i<600&&!tiles.tilesLoaded;i++)await wait(50);await wait(400);
          if(!tiles.tilesLoaded)throw Error(`${name} did not settle`);}
        function selected(){return tiles._selectedTiles.map(tile=>({url:tile.content.url,
          level:level(tile.content.url,octree),features:tile.content.featuresLength,triangles:tile.content.trianglesLength}));}
        frame(32);await settle();const coarse=selected();
        tiles.maximumScreenSpaceError=1e-7;frame(3);await settle();const fine=selected();
        const levels=new Set();
        function walk(tile){
          const implicit=tile.implicitTileset || tile.implicitSubtree?._implicitTileset;
          if(tile.implicitCoordinates){
            const base=level(implicit?.baseResource?.url,octree)||0;
            levels.add(base+tile.implicitCoordinates.level);
          }
          for(const content of tile._header.contents || (tile._header.content?[tile._header.content]:[])) {
            const value=level(content.uri,octree);if(value!==undefined)levels.add(value);
          }
          for(const child of tile.children)walk(child);
        }
        walk(tiles.root);
        let picked;
        for(const tile of tiles._selectedTiles.slice()){
          const screen=C.SceneTransforms.worldToWindowCoordinates(v.scene,tile.boundingVolume.boundingSphere.center);
          const feature=screen&&v.scene.pick(screen,3,3);
          if(feature){picked=feature.getPropertyIds?Object.fromEntries(feature.getPropertyIds().map(key=>[key,String(feature.getProperty(key))])):{mesh:true};break;}
        }
        outputs.push({name,coarse,fine,levels:[...levels].sort((a,b)=>a-b),picked,failures:window.failures.slice()});
        tiles.show=false;
      }
      return {version:C.VERSION,outputs};
    });
    console.log(JSON.stringify({...results,browserErrors:errors},null,2));
    assert.deepEqual(errors,[]);
    for(const result of results.outputs){
      assert.deepEqual(result.failures,[]);assert.ok(result.coarse.length);assert.ok(result.fine.length);
      const maximum=Math.max(...result.levels);assert.ok(maximum>=4,'Fixture did not cross a boundary');
      assert.deepEqual(result.levels,Array.from({length:maximum+1},(_,i)=>i));
      assert.ok(Math.max(...result.fine.map(t=>t.level))>Math.max(...result.coarse.map(t=>t.level)));
      assert.ok(result.picked,`${result.name} was not picked`);
      if(result.name==='point-cloud'){
        assert.equal(result.fine.reduce((sum,t)=>sum+t.features,0),257);
        assert.ok(Object.hasOwn(result.picked,'source_index'));
      } else assert.equal(result.fine.reduce((sum,t)=>sum+t.triangles,0),1152);
    }
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
