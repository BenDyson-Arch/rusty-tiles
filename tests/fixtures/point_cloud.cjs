// Optional Cesium IIFE acceptance against a converted native point-cloud preview.
// NODE_PATH exposes Playwright; Cesium/data are served locally, without downloads.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.goto(process.argv[2] || 'http://127.0.0.1:9271');
    const inspect = async () => {
      await page.waitForFunction(() => window.pointCloud || window.failures?.length, null, {timeout: 30000});
      return page.evaluate(async () => {
        const C = Cesium, viewer = window.viewer, cloud = window.pointCloud;
        if (!cloud) throw new Error(window.failures.join('\n'));
        viewer.scene.globe.show = false;
        viewer.scene.skyAtmosphere.show = false;
        viewer.scene.skyBox.show = false;
        cloud.style = new C.Cesium3DTileStyle({color: "color('cyan')", pointSize: 12});
        const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
        const frame = multiplier => {
          viewer.camera.viewBoundingSphere(cloud.boundingSphere, new C.HeadingPitchRange(-.8, -.65, cloud.boundingSphere.radius * multiplier));
          viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
        };
        async function settle() {
          await wait(300);
          for (let i = 0; i < 100 && !cloud.tilesLoaded; i++) await wait(50);
          await wait(300);
          if (!cloud.tilesLoaded) throw new Error('Point cloud did not finish refinement');
        }
        // Private selection fields are confined to this diagnostic probe.
        const selection = () => (cloud._selectedTiles || []).map(tile => ({
          uri: tile.content.url, features: tile.content.featuresLength,
        }));
        frame(100); await settle(); const coarse = selection();
        frame(3); await settle(); const fine = selection();
        let picked;
        for (const tile of cloud._selectedTiles || []) {
          if (!tile.content.url) continue;
          const bytes = await (await fetch(tile.content.url)).arrayBuffer();
          const header = new DataView(bytes), length = header.getUint32(12, true);
          const doc = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 20, length)));
          const accessor = doc.accessors[doc.meshes[0].primitives[0].attributes.POSITION];
          const view = doc.bufferViews[accessor.bufferView];
          const positions = new Float32Array(bytes, 28 + length + (view.byteOffset || 0), accessor.count * 3);
          const node = doc.nodes[doc.scenes[doc.scene || 0].nodes[0]];
          const translation = node.translation || [0, 0, 0];
          for (let i = 0; i < accessor.count; i++) {
            const world = C.Matrix4.multiplyByPoint(tile.computedTransform,
              new C.Cartesian3(positions[i*3]+translation[0], -positions[i*3+2]-translation[2], positions[i*3+1]+translation[1]), new C.Cartesian3());
            const screen = C.SceneTransforms.worldToWindowCoordinates(viewer.scene, world);
            const feature = screen && viewer.scene.pick(screen, 1, 1);
            if (!feature?.getPropertyIds) continue;
            const properties = Object.fromEntries(feature.getPropertyIds().map(name => [name,
              String(feature.getProperty(name))]));
            // Invented fixture includes scalar temperature and exact raw XYZ/RGB.
            if (Object.hasOwn(properties, 'source_index')) { picked = properties; break; }
          }
          if (picked) break;
        }
        return {version: C.VERSION, coarse, fine, picked, failures: window.failures.slice()};
      });
    };
    const first = await inspect();
    // Bypass browser cache and load the real preview/tileset again.
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.setCacheDisabled', {cacheDisabled: true});
    await page.reload({waitUntil: 'load'});
    const refreshed = await inspect();
    const report = {first, refreshed, browserErrors: errors};
    console.log(JSON.stringify(report, null, 2));
    assert.equal(errors.length, 0);
    for (const result of [first, refreshed]) {
      assert.equal(result.version, '1.143.0');
      assert.equal(result.failures.length, 0);
      assert.ok(result.coarse.length > 0);
      const coarse = result.coarse.reduce((sum, tile) => sum + tile.features, 0);
      const fine = result.fine.reduce((sum, tile) => sum + tile.features, 0);
      assert.ok(fine > coarse, `LOD did not refine: ${coarse} → ${fine}`);
      assert.equal(fine, 257, 'Full detail did not retain every fixture point');
      assert.ok(result.picked, 'No full-detail source point rendered/picked');
      for (const name of ['X','Y','Z','source_x','source_y','source_z','intensity','classification','red','green','blue','temperature']) {
        assert.ok(Object.hasOwn(result.picked, name), `Missing picked property ${name}`);
      }
      const id = Number(result.picked.source_index);
      assert.ok(Number.isInteger(id) && id >= 0 && id < 257);
      assert.equal(Number(result.picked.X), (id % 17) * 713);
      assert.equal(Number(result.picked.Y), Math.floor(id / 17) * 627);
      assert.equal(Number(result.picked.Z), Math.round(Math.sin(id) * 2000));
      assert.equal(Number(result.picked.source_x), 500000 + Number(result.picked.X) * .001);
      assert.equal(Number(result.picked.source_y), 5000000 + Number(result.picked.Y) * .001);
      assert.equal(Number(result.picked.source_z), 80 + Number(result.picked.Z) * .001);
      assert.equal(Number(result.picked.temperature), id / 7);
      assert.equal(Number(result.picked.classification), id % 20);
      for (const [name, factor] of [['red',233],['green',431],['blue',717]]) {
        assert.equal(Number(result.picked[name]), (id * factor) % 65536);
      }
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
