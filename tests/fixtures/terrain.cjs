// Cesium 1.146.0 acceptance: URL and independently decoded GLB oracle JSON.
// Oracle: {samples:[{longitude,latitude,height,tolerance}],outside:[{longitude,latitude}],minimumLeaves:2,imagery:true}.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const {createHash} = require('node:crypto');
const {chromium} = require('playwright');
const oracle = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
assert(oracle.samples.some(p => p.height < 0), 'negative-height oracle required');
assert(oracle.samples.some(p => p.height > 0), 'positive-height oracle required');
assert(oracle.outside.length, 'outside-domain oracle required');
assert(oracle.outside.some(p => Number.isFinite(p.underlyingMeshHeight)), 'decoded mesh boundary hit required');
(async () => {
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
    headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader']});
  try {
    const page = await browser.newPage({viewport: {width: 1100, height: 800}});
    const errors = [], external = [], requests = [];
    page.on('pageerror', e => errors.push(String(e)));
    page.on('request', r => requests.push(r.url()));
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (!['127.0.0.1', 'localhost'].includes(url.hostname)) {
        external.push(url.href); return route.abort();
      }
      return route.continue();
    });
    const inspect = async () => {
      await page.waitForFunction(() => window.terrainSurface && window.terrain && window.loaded,
        null, {timeout: 60000});
      await page.getByRole('button', {name: 'Terrain extent', exact: true}).click();
      return page.evaluate(async oracle => {
        const C = Cesium, scene = viewer.scene, surface = window.terrainSurface;
        const samples = oracle.samples.map((p, i) => C.Cartographic.fromDegrees(p.longitude, p.latitude, i % 2 ? 20000 : -25000));
        const outside = oracle.outside.map(p => C.Cartographic.fromDegrees(p.longitude, p.latitude));
        const heights = await surface.sampleHeights(samples);
        const absent = await surface.sampleHeights(outside);
        const borderControls = oracle.outside.filter(p => Number.isFinite(p.underlyingMeshHeight));
        const rawBorder = await scene.sampleHeightMostDetailed(borderControls.map(p =>
          C.Cartographic.fromDegrees(p.longitude, p.latitude)));
        const positions = oracle.samples.map(p => C.Cartesian3.fromDegrees(p.longitude, p.latitude, -25000));
        const clamped = await surface.clampPositions(positions);
        const missingClamp = await surface.clampPositions(oracle.outside.map(p =>
          C.Cartesian3.fromDegrees(p.longitude, p.latitude, 2000)));
        // An unrelated scene object must not turn this into a highest-scene-surface query.
        const p = oracle.samples[0];
        const obstacle = viewer.entities.add({position: C.Cartesian3.fromDegrees(p.longitude, p.latitude, p.height + 100),
          box: {dimensions: new C.Cartesian3(20, 20, 20), material: C.Color.RED}});
        scene.render();
        const isolated = await surface.sampleHeights([C.Cartographic.fromDegrees(p.longitude, p.latitude)]);
        viewer.entities.remove(obstacle);
        const height = p => typeof p === 'number' ? p : p?.height;
        return {version: C.VERSION, supported: scene.sampleHeightSupported && scene.clampToHeightSupported,
          collision: terrain.enableCollision, globeHidden: !scene.globe.show,
          heights: heights.map(height), absent: absent.map(height), isolated: isolated.map(height),
          rawBorderHeights: rawBorder.map(height),
          clamped: clamped.map(p => p && C.Cartographic.fromCartesian(p).height),
          missingClamp: missingClamp.map(p => !!p), failures: window.failures};
      }, oracle);
    };
    await page.goto(process.argv[2]);
    const first = await inspect();
    let imageryProof;
    if (oracle.imagery) {
      await page.waitForFunction(() => terrain.imageryLayers.length > 0);
      await page.evaluate(() => {
        viewer.clock.shouldAnimate = false;
        viewer.scene.debugShowFramesPerSecond = false;
        viewer.scene.screenSpaceCameraController.enableInputs = false;
        for (const layer of [window.tiles, window.annotations, window.pointCloud]) if (layer) layer.show = false;
        terrain.imageryLayers.get(0).show = false;
        viewer.scene.requestRender();
      });
      await page.waitForTimeout(500);
      const bare = await page.locator('canvas').first().screenshot();
      await page.evaluate(() => {terrain.imageryLayers.get(0).show = true; viewer.scene.requestRender();});
      await page.waitForTimeout(2000);
      const draped = await page.locator('canvas').first().screenshot();
      const pixels = await page.evaluate(async images => {
        const decoded = [];
        for (const base64 of images) {
          const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
          const image = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
          const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
          const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
          decoded.push(context.getImageData(0, 0, image.width, image.height).data); image.close();
        }
        let changed = 0, orangeBefore = 0, orangeAfter = 0;
        const orange = (p, i) => p[i] > 1.5 * p[i + 1] && p[i + 1] > 1.5 * p[i + 2] && p[i + 1] > 20;
        for (let i = 0; i < decoded[0].length; i += 4) {
          if (orange(decoded[0], i)) orangeBefore++;
          if (orange(decoded[1], i)) orangeAfter++;
          if (Math.abs(decoded[0][i] - decoded[1][i]) + Math.abs(decoded[0][i + 1] - decoded[1][i + 1])
            + Math.abs(decoded[0][i + 2] - decoded[1][i + 2]) > 30) changed++;
        }
        return {changed, orangeBefore, orangeAfter};
      }, [bare.toString('base64'), draped.toString('base64')]);
      assert(pixels.changed > 100, 'imagery must change at least 100 rendered pixels');
      assert(pixels.orangeAfter > pixels.orangeBefore + 100, 'fixture orange imagery must appear on terrain');
      assert(requests.some(url => /\/imagery\/.*\.(png|jpg|jpeg|webp)(?:\?|$)/i.test(url)), 'local imagery must be requested');
      imageryProof = {...pixels, bareSha256: createHash('sha256').update(bare).digest('hex'),
        drapedSha256: createHash('sha256').update(draped).digest('hex')};
    }
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.setCacheDisabled', {cacheDisabled: true});
    await page.reload();
    const refreshed = await inspect();
    for (const result of [first, refreshed]) {
      assert.equal(result.version, '1.146.0');
      assert.equal(result.supported, true); assert.equal(result.collision, true);
      assert.equal(result.globeHidden, true); assert.deepEqual(result.failures, []);
      oracle.samples.forEach((p, i) => {
        for (const actual of [result.heights[i], result.clamped[i]]) {
          assert(Number.isFinite(actual));
          assert(Math.abs(actual - p.height) <= p.tolerance, `decoded GLB surface expected ${p.height}, got ${actual}`);
        }
      });
      assert(Math.abs(result.isolated[0] - oracle.samples[0].height) <= oracle.samples[0].tolerance);
      oracle.outside.filter(p => Number.isFinite(p.underlyingMeshHeight)).forEach((p, i) => {
        assert(Number.isFinite(result.rawBorderHeights[i]), 'underlying mesh boundary must remain queryable');
        assert(Math.abs(result.rawBorderHeights[i] - p.underlyingMeshHeight) <= p.tolerance);
      });
      assert(result.absent.every(h => h == null));
      assert(result.missingClamp.every(p => !p));
    }
    const glbs = new Set(requests.filter(url => /\/terrain\/.*\.glb(?:\?|$)/.test(url)));
    assert(glbs.size >= (oracle.minimumLeaves || 2), 'multiple terrain patches must load');
    assert.deepEqual(errors, []); assert.deepEqual(external, []);
    console.log(JSON.stringify({first, refreshed, terrainLeaves: glbs.size, imageryProof, browserErrors: errors}, null, 2));
  } finally {await browser.close();}
})().catch(error => {console.error(error); process.exitCode = 1;});
