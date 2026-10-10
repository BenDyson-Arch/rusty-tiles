const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM, headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport: {width: 1100, height: 800}});
    const external = [], errors = [], responses = [], consoleErrors = [];
    page.on('console', message => {if (message.type() === 'error') consoleErrors.push(message.text());});
    page.on('pageerror', e => errors.push(String(e)));
    page.on('response', r => responses.push({url: r.url(), status: r.status()}));
    await page.route('**/*', route => {
      if (new URL(route.request().url()).hostname !== '127.0.0.1') {
        external.push(route.request().url()); return route.abort();
      }
      return route.continue();
    });
    await page.goto(process.argv[2]);
    await page.waitForFunction(() => window.viewer && window.Cesium);
    const result = await page.evaluate(async () => {
      const C = window.Cesium, viewer = window.viewer, scene = viewer.scene;
      const plans = await (await fetch('/plan.json')).json(), runs = [];
      scene.highDynamicRange = false;
      const vector = values => new C.Cartesian3(...values);
      const frame = () => new Promise(resolve => {
        const remove = scene.postRender.addEventListener(() => {remove(); resolve();});
        scene.requestRender();
      });
      for (const plan of plans) {
        viewer.camera.frustum.near = 0.1;
        viewer.camera.setView({destination: vector(plan.camera.position), orientation: {
          direction: vector(plan.camera.direction), up: vector(plan.camera.up)}});
        for (const variant of ['good', 'wrong-root', 'missing-alias', 'zero-omission-error']) {
          const tileFailures = [];
          const tiles = await C.Cesium3DTileset.fromUrl('/'+plan.placement+'/'+variant+'/tileset.json',
            {maximumScreenSpaceError: 1, skipLevelOfDetail: false});
          tiles.tileFailed.addEventListener(error => tileFailures.push(String(error.message)));
          scene.primitives.add(tiles);
          let loaded = false;
          for (let i = 0; i < 200; i++) {
            await frame();
            // Missing glTF buffers may remain in processing without firing tileFailed in this Cesium.
            // Observe the real failed request; the host independently verifies the exact 404 response.
            const missingAlias404 = performance.getEntriesByType('resource').some(entry =>
              entry.name.includes('/'+plan.placement+'/missing-alias/model/alias%25.bin') && entry.responseStatus === 404);
            const ready = variant === 'missing-alias' ? missingAlias404 :
              tiles.tilesLoaded && (variant !== 'good' || tiles._statistics.numberOfCommands > 0);
            if (ready) {loaded = true; await frame(); await frame(); break;}
            await new Promise(resolve => setTimeout(resolve, 15));
          }
          const triangles = tiles._selectedTiles.reduce((n, tile) => n+tile.content.trianglesLength, 0);
          const samples = plan.targets.map(target => {
            const screen = C.SceneTransforms.worldToWindowCoordinates(scene, vector(target));
            const picked = screen && scene.pick(screen, 1, 1);
            const owns = !!picked && (picked.primitive === tiles || picked.tileset === tiles || picked.content?.tile?.tileset === tiles);
            return {target, screen: screen ? [screen.x, screen.y] : null, picked: owns};
          });
          runs.push({placement: plan.placement, variant, loaded, triangles, tileFailures,
            tilesLoaded: tiles.tilesLoaded, statistics: {...tiles._statistics},
            rootSphere: tiles.root.boundingSphere, rootTransform: C.Matrix4.toArray(tiles.root.transform),
            modelMatrixIsIdentity: C.Matrix4.equals(tiles.modelMatrix, C.Matrix4.IDENTITY), samples});
          scene.primitives.remove(tiles);
        }
      }
      return {cesiumVersion: C.VERSION, runs};
    });
    console.log(JSON.stringify({...result, browserVersion: browser.version(), responses, errors, external, consoleErrors}, null, 2));
    for (const run of result.runs) {
      assert.ok(run.loaded, JSON.stringify({run, responses, consoleErrors})); assert.ok(run.modelMatrixIsIdentity);
      if (run.variant === 'good') {
        assert.equal(run.triangles, 2); assert.deepEqual(run.tileFailures, []);
        assert.ok(run.samples.every(sample => sample.picked), JSON.stringify(run));
      } else if (run.variant === 'wrong-root') {
        assert.ok(run.samples.every(sample => !sample.picked), JSON.stringify(run));
      } else if (run.variant === 'zero-omission-error') {
        assert.equal(run.triangles, 0); assert.equal(run.statistics.visited, 0);
        assert.equal(run.statistics.numberOfCommands, 0);
        assert.ok(run.samples.every(sample => !sample.picked), JSON.stringify(run));
        assert.deepEqual(run.tileFailures, []);
      } else {
        assert.equal(run.triangles, 0); assert.equal(run.statistics.numberOfCommands, 0);
        assert.ok(run.samples.every(sample => !sample.picked), 'omitted resource alias prevents content rendering');
      }
    }
    for (const placement of ['local', 'wgs84']) {
      for (const resource of ['geometry%20%CE%B2.bin', 'alias%25.bin', 'texture%20%25%20%CE%B2.png'])
        assert.ok(responses.some(r => r.url.includes('/'+placement+'/good/model/'+resource) && r.status === 200), resource);
      assert.ok(responses.some(r => r.url.includes('/'+placement+'/missing-alias/model/alias%25.bin') && r.status === 404));
      assert.ok(!responses.some(r => r.url.includes('/'+placement+'/zero-omission-error/model/')),
        'zero top-level error must prevent unchanged model resource requests');
    }
    assert.deepEqual(errors, []); assert.deepEqual(external, []);
  } finally {await browser.close();}
})().catch(error => {console.error(error); process.exitCode = 1;});
