// The README's actual pyramid must render and be pickable after a hard refresh.
const assert = require('node:assert/strict');
const {chromium} = require('playwright');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({viewport: {width: 1100, height: 800}});
    const errors = [], external = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (!['127.0.0.1', 'localhost'].includes(url.hostname)) {
        external.push(url.href);
        return route.abort();
      }
      return route.continue();
    });
    const inspect = async () => {
      await page.waitForFunction(() => window.tiles, null, {timeout: 30000});
      await page.getByRole('button', {name: '3D mesh extent', exact: true}).click();
      return page.evaluate(async () => {
        const mesh = window.tiles, viewer = window.viewer;
        viewer.scene.globe.show = false;
        let picked;
        for (let i = 0; i < 200 && !picked; i++) {
          await new Promise(resolve => setTimeout(resolve, 50));
          viewer.scene.render();
          if (!mesh.tilesLoaded) continue;
          const center = Cesium.SceneTransforms.worldToWindowCoordinates(viewer.scene, mesh.boundingSphere.center);
          picked = center && viewer.scene.pick(center, 3, 3);
        }
        return {version: Cesium.VERSION, loaded: mesh.tilesLoaded, picked: !!picked,
          failures: window.failures.slice()};
      });
    };
    await page.goto(process.argv[2]);
    const first = await inspect();
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.setCacheDisabled', {cacheDisabled: true});
    await page.reload({waitUntil: 'load'});
    const refreshed = await inspect();
    console.log(JSON.stringify({first, refreshed, browserErrors: errors, externalRequests: external}, null, 2));
    for (const result of [first, refreshed]) {
      assert.equal(result.version, '1.146.0');
      assert.equal(result.loaded, true);
      assert.equal(result.picked, true);
      assert.deepEqual(result.failures, []);
    }
    assert.deepEqual(errors, []);
    assert.deepEqual(external, []);
  } finally {
    await browser.close();
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
