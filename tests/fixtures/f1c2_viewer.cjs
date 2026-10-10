// Real Cesium picking: source keys are independently authored camera targets.
// No custom shader, modelMatrix repair or feature-table manipulation.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM, headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport: {width: 1100, height: 800}});
    const errors = [], failed = [], external = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.on('requestfailed', request => failed.push(request.url()));
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (url.hostname !== '127.0.0.1') { external.push(url.href); return route.abort(); }
      return route.continue();
    });
    await page.goto(process.argv[2]);
    await page.waitForFunction(() => window.viewer && window.Cesium);
    const result = await page.evaluate(async () => {
      const C = window.Cesium, viewer = window.viewer, scene = viewer.scene;
      const plan = await (await fetch('/plan.json')).json(), runs = [];
      const vector = a => new C.Cartesian3(...a);
      scene.highDynamicRange = false;
      viewer.camera.frustum.near = 0.1;
      viewer.camera.setView({destination: vector(plan.camera.destination), orientation: {
        direction: vector(plan.camera.direction), up: vector(plan.camera.up)}});
      async function frame() {
        await new Promise(resolve => {
          const remove = scene.postRender.addEventListener(() => {remove(); resolve();});
          scene.requestRender();
        });
      }
      async function settle(tiles) {
        for (let i = 0; i < 250; i++) {
          await frame();
          if (tiles.tilesLoaded && tiles._statistics.numberOfCommands > 0) {await frame(); await frame(); return;}
          await new Promise(resolve => setTimeout(resolve, 15));
        }
        throw new Error('picking tileset failed to settle');
      }
      for (const mode of ['default', 'source_primitive', 'source_triangle']) {
        for (const variant of ['good', 'primitive-row-swap', 'triangle-row-swap']) {
          const options = {maximumScreenSpaceError: 1, skipLevelOfDetail: false};
          if (mode !== 'default') options.featureIdLabel = mode;
          const tiles = await C.Cesium3DTileset.fromUrl('/' + variant + '/tileset.json', options);
          scene.primitives.add(tiles); await settle(tiles);
          const ordinaryTriangles = tiles._selectedTiles.reduce((n, tile) => n + tile.content.trianglesLength, 0);
          const label = mode === 'default' ? 'source_primitive' : mode;
          const samples = plan.targets.map(target => {
            const screen = C.SceneTransforms.worldToWindowCoordinates(scene, vector(target.world));
            const picked = screen && scene.pick(screen, 1, 1);
            const owns = picked instanceof C.Cesium3DTileFeature && picked.tileset === tiles;
            const properties = {};
            if (owns) for (const name of Object.keys(target[label])) properties[name] = picked.getProperty(name);
            return {world: target.world, screen: screen ? [screen.x, screen.y] : null, picked: owns,
                    expected: target[label], properties, pickedType: picked?.constructor?.name,
                    matches: owns && JSON.stringify(properties) === JSON.stringify(target[label])};
          });
          runs.push({mode, variant, ordinaryTriangles, modelMatrixIsIdentity: C.Matrix4.equals(tiles.modelMatrix, C.Matrix4.IDENTITY), samples});
          scene.primitives.remove(tiles);
        }
      }
      return {version: C.VERSION, independentCamera: plan.camera, expectedTriangles: plan.triangleCount, runs};
    });
    assert.equal(result.version, '1.146.0');
    for (const run of result.runs) {
      assert.equal(run.modelMatrixIsIdentity, true);
      assert.equal(run.ordinaryTriangles, result.expectedTriangles);
      assert.ok(run.samples.every(sample => sample.picked), run.mode + '/' + run.variant + ' all exact interior targets pick: ' + JSON.stringify(run));
      const shouldFail = (run.mode === 'source_triangle' && run.variant === 'triangle-row-swap') ||
        (run.mode !== 'source_triangle' && run.variant === 'primitive-row-swap');
      if (shouldFail) assert.ok(run.samples.some(sample => !sample.matches), run.mode + ' sensitive ID swap');
      else assert.ok(run.samples.every(sample => sample.matches), run.mode + '/' + run.variant + ' exact source properties');
    }
    assert.deepEqual(errors, []); assert.deepEqual(failed, []); assert.deepEqual(external, []);
    console.log(JSON.stringify({...result, browserVersion: browser.version(), pageErrors: errors,
      failedRequests: failed, externalRequests: external}, null, 2));
  } finally {await browser.close();}
})().catch(error => {console.error(error); process.exitCode = 1;});
