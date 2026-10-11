// Load deep native fixtures written as the bounded implicit forest, cross its
// explicit-skeleton/implicit-root and explicit-document boundaries, check every
// instantiated producer level, full leaf selection, and pick mesh/point content.
// Usage: NODE_PATH=... node implicit_levels.cjs PREVIEW_URL [--require-documents]
// --require-documents fails unless some fixture loads an external document.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
const requireDocuments = process.argv.includes('--require-documents');
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
      // Producer-hierarchy level. Cesium gives an implicit root a placeholder
      // tile whose child is the root's level-0 tile, and loads a link tile's
      // document root as the link's child. Each pair is one producer tile.
      function level(tile){
        let value=0;
        for(let t=tile;t.parent;t=t.parent)if(!t.parent.hasImplicitContent&&!t.parent.hasTilesetContent)value++;
        return value;
      }
      for(const [name,tiles] of [['mesh',window.tiles],['point-cloud',window.pointCloud]]) {
        tiles.show=true;tiles.maximumScreenSpaceError=8;
        function frame(multiplier){v.camera.viewBoundingSphere(tiles.boundingSphere,
          new C.HeadingPitchRange(0,-Math.PI/2,tiles.boundingSphere.radius*multiplier));v.camera.lookAtTransform(C.Matrix4.IDENTITY);}
        async function settle(){await wait(250);for(let i=0;i<600&&!tiles.tilesLoaded;i++)await wait(50);await wait(400);
          if(!tiles.tilesLoaded)throw Error(`${name} did not settle`);}
        // A forest root's single subtree instantiates all of its tiles, so a
        // selected tile without children is a producer leaf.
        function selected(){return tiles._selectedTiles.map(tile=>({url:tile.content.url,level:level(tile),
          leaf:tile.children.length===0,features:tile.content.featuresLength,triangles:tile.content.trianglesLength}));}
        frame(32);await settle();const coarse=selected();
        tiles.maximumScreenSpaceError=1e-7;frame(3);await settle();const fine=selected();
        const levels=new Set();let implicitRoots=0,nestedRoots=0,documents=0,implicitTiles=0;
        function walk(tile){
          levels.add(level(tile));
          if(tile.hasImplicitContent){implicitRoots++;if(level(tile)>0)nestedRoots++;}
          if(tile.hasTilesetContent)documents++;
          if(tile.implicitCoordinates){
            implicitTiles++;
            // An implicit tile's level is its root's level plus its local level.
            let root=tile;while(root.parent?.implicitCoordinates)root=root.parent;
            if(level(root)+tile.implicitCoordinates.level!==level(tile))throw Error(`${name}: implicit level mismatch at ${tile.content?.url}`);
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
        outputs.push({name,coarse,fine,levels:[...levels].sort((a,b)=>a-b),implicitRoots,nestedRoots,documents,implicitTiles,
          picked,failures:window.failures.slice()});
        tiles.show=false;
      }
      return {version:C.VERSION,outputs};
    });
    console.log(JSON.stringify({...results,browserErrors:errors},null,2));
    assert.deepEqual(errors,[]);
    for(const result of results.outputs){
      assert.deepEqual(result.failures,[]);assert.ok(result.coarse.length);assert.ok(result.fine.length);
      const maximum=Math.max(...result.levels);assert.ok(maximum>=4,`${result.name} did not reach below the skeleton`);
      assert.deepEqual(result.levels,Array.from({length:maximum+1},(_,i)=>i));
      // Implicit roots hang below explicit skeleton tiles, and their tiles load.
      assert.ok(result.nestedRoots>0,`${result.name} did not cross an implicit-root boundary`);
      assert.ok(result.implicitTiles>0,`${result.name} instantiated no implicit tile`);
      assert.ok(Math.max(...result.fine.map(t=>t.level))>Math.max(...result.coarse.map(t=>t.level)));
      assert.equal(Math.max(...result.fine.map(t=>t.level)),maximum,`${result.name} fine view did not reach the deepest level`);
      assert.ok(result.fine.every(t=>t.leaf),`${result.name} fine view selected a non-leaf tile`);
      assert.ok(result.picked,`${result.name} was not picked`);
      if(result.name==='point-cloud'){
        assert.equal(result.fine.reduce((sum,t)=>sum+t.features,0),257);
        assert.ok(Object.hasOwn(result.picked,'source_index'));
      } else assert.equal(result.fine.reduce((sum,t)=>sum+t.triangles,0),1152);
    }
    if(requireDocuments)assert.ok(results.outputs.some(r=>r.documents>0),'no fixture crossed an external document');
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
