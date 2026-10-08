// Optional acceptance against CesiumJS 1.146.0 and Chromium. Run the native
// metadata tests with RUSTY_TILES_METADATA_ACCEPTANCE_DIR, serve mesh-false-true
// and point-false with `preview`, then pass its URL to this probe.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport: {width: 1100, height: 800}});
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.goto(process.argv[2]);
    await page.waitForFunction(() => window.tiles && window.pointCloud || window.failures?.length, null, {timeout: 30000});
    const result = await page.evaluate(async () => {
      const C = Cesium, viewer = window.viewer, mesh = window.tiles, cloud = window.pointCloud;
      if (!mesh || !cloud) throw new Error(window.failures.join('\n'));
      viewer.scene.globe.show = false;
      viewer.scene.skyAtmosphere.show = false;
      viewer.scene.skyBox.show = false;
      viewer.scene.sun.show = false;
      mesh.backFaceCulling = false;
      const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
      async function settle(tiles) {
        await wait(300);
        for (let i = 0; i < 100 && !tiles.tilesLoaded; i++) await wait(50);
        await wait(300);
        if (!tiles.tilesLoaded) throw new Error('Tiles did not finish loading');
      }
      function camera(center, distance) {
        const destination = new C.Cartesian3(center.x, center.y - distance, center.z + distance * .6);
        const direction = C.Cartesian3.normalize(C.Cartesian3.subtract(center, destination, new C.Cartesian3()), new C.Cartesian3());
        viewer.camera.setView({destination, orientation: {direction, up: C.Cartesian3.UNIT_Z}});
      }
      cloud.show = false;
      mesh.style = new C.Cesium3DTileStyle({color: {conditions: [
        ['${node_index} === 0', "color('red')"], ['${node_index} === 1', "color('lime')"], ['true', "color('blue')"]]}});
      camera(mesh.boundingSphere.center, mesh.boundingSphere.radius * 3);
      await settle(mesh);
      const buildings = [];
      for (const [id, position] of [[0,[-3+1/3,0,1/3]],[1,[3+1/3,0,1/3]],[3,[1/3,-3,1/3]]]) {
        const screen = C.SceneTransforms.worldToWindowCoordinates(viewer.scene, C.Cartesian3.fromArray(position));
        const feature = screen && viewer.scene.pick(screen, 3, 3);
        if (!feature?.getProperty) throw new Error(`Building ${id} was not picked`);
        buildings.push({id: Number(feature.getProperty('node_index')), name: feature.getProperty('name'), color: feature.color.toCssColorString()});
      }
      mesh.show = false;
      cloud.show = true;
      cloud.pointCloudShading.attenuation = false; cloud.colorBlendMode = C.Cesium3DTileColorBlendMode.REPLACE;
      cloud.style = new C.Cesium3DTileStyle({color: "${classification} === 42 && ${return_number} === 1 && ${intensity} >= 0 ? color('cyan') : color('red')", pointSize: 12});
      camera(cloud.boundingSphere.center, cloud.boundingSphere.radius * 3);
      await settle(cloud);
      let point;
      for (const tile of cloud._selectedTiles || []) {
        if (!tile.content.url) continue;
        const bytes = await (await fetch(tile.content.url)).arrayBuffer();
        const length = new DataView(bytes).getUint32(12, true);
        const doc = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes,20,length)));
        const primitive = doc.meshes[0].primitives[0];
        if (!primitive.extensions.EXT_structural_metadata?.propertyAttributes) throw new Error('Missing point property attributes');
        const accessor = doc.accessors[primitive.attributes.POSITION];
        const view = doc.bufferViews[accessor.bufferView];
        const positions = new Float32Array(bytes, 28 + length + (view.byteOffset || 0), accessor.count * 3);
        const t = doc.nodes[doc.scenes[doc.scene || 0].nodes[0]].translation || [0,0,0];
        for (let i = 0; i < accessor.count; i++) {
          const world = C.Matrix4.multiplyByPoint(tile.computedTransform,
            new C.Cartesian3(positions[i*3]+t[0], -positions[i*3+2]-t[2], positions[i*3+1]+t[1]), new C.Cartesian3());
          const screen = C.SceneTransforms.worldToWindowCoordinates(viewer.scene,world);
          const feature = screen && viewer.scene.pick(screen,1,1);
          if (!feature?.getProperty || feature.getProperty('source_index') === undefined) continue;
          point = Object.fromEntries(['source_index','classification','intensity','return_number'].map(name => [name,Number(feature.getProperty(name))]));
          viewer.scene.render();
          point.pixel = Array.from(viewer.scene.context.readPixels({x:Math.floor(screen.x), y:viewer.scene.drawingBufferHeight - Math.floor(screen.y) - 1, width:1, height:1}));
          break;
        }
        if (point) break;
      }
      return {version: C.VERSION, buildings, point, failures: window.failures};
    });
    console.log(JSON.stringify({...result, browserErrors: errors}, null, 2));
    assert.equal(result.version, '1.146.0');
    assert.deepEqual(result.failures, []);
    assert.deepEqual(errors, []);
    assert.deepEqual(result.buildings.map(b => b.id), [0,1,3]);
    assert.deepEqual(result.buildings.map(b => b.name), ['Building 🦉','Building 🦉','node_3']);
    assert.deepEqual(result.buildings.map(b => b.color), ['rgb(255,0,0)','rgb(0,255,0)','rgb(0,0,255)']);
    assert.ok(result.point, 'No point was picked');
    assert.equal(result.point.classification, 42);
    assert.equal(result.point.return_number, 1);
    assert.equal(result.point.intensity, result.point.source_index * 13);
    assert.equal(result.point.pixel[0], 0);
    assert.ok(result.point.pixel[1] > 0 && result.point.pixel[1] === result.point.pixel[2], 'Point style did not render cyan');
  } finally {await browser.close();}
})().catch(error => {console.error(error); process.exitCode = 1;});
