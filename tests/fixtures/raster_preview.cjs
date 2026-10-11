// An R2 pyramid with tile-aligned TileJSON bounds must load in `rusty-tiles
// preview` (Cesium 1.146.0) without page errors, and its pixels must render.
// usage: node raster_preview.cjs PREVIEW_URL [W,S,E,N]   (preview started with --imagery only)
// The optional rectangle (degrees) is a transparent hole: samples inside it,
// inset 10%, must show the bare globe; samples outside it, padded 10%, imagery.
const assert = require('node:assert/strict');
const {chromium} = require('playwright');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({viewport: {width: 1024, height: 768}});
    const errors = [], external = [], failed = [];
    let imageryTiles = 0;
    page.on('pageerror', error => errors.push(String(error)));
    page.on('console', message => message.type() === 'error' && errors.push(message.text()));
    page.on('response', response => {
      if (response.status() >= 400) failed.push(`${response.status()} ${response.url()}`);
      else if (new URL(response.url()).pathname.startsWith('/imagery/tiles/')) imageryTiles++;
    });
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (!['127.0.0.1', 'localhost'].includes(url.hostname)) {
        external.push(url.href);
        return route.abort();
      }
      return route.continue();
    });
    await page.goto(process.argv[2]);
    await page.waitForFunction(() => window.loaded || window.failures?.length, null, {timeout: 120000});
    const early = await page.evaluate(() => ({version: Cesium.VERSION, failures: window.failures.slice()}));
    if (early.failures.length || errors.length) {
      console.log(JSON.stringify({...early, browserErrors: errors, failedRequests: failed}, null, 2));
      throw new Error(`preview failed before the render probe: ${early.failures.concat(errors).join('; ')}`);
    }
    // Render probe: a 9x9 grid inside the imagery bounds (inset 10%) must show
    // the imagery and, with the layer hidden, the bare globe colour. The bare
    // control also proves the canvas readback is real.
    const hole = process.argv[3] ? process.argv[3].split(',').map(Number) : null;
    const result = await page.evaluate(async hole => {
      const C = Cesium, viewer = window.viewer, scene = viewer.scene;
      const metadata = await (await fetch(JSON.parse(await (await fetch('/config.json')).text()).imagery)).json();
      const [w, s, e, n] = metadata.bounds;
      const base = C.Color.fromCssColorString('#59646d');
      const settle = async () => {
        for (let stable = 0, i = 0; stable < 5 && i < 600; i++) {
          await new Promise(resolve => setTimeout(resolve, 20));
          viewer.render();
          stable = scene.globe.tilesLoaded ? stable + 1 : 0;
        }
      };
      const sample = async () => {
        await settle();
        viewer.render();  // read back in the same task as the frame
        const canvas = viewer.canvas, copy = document.createElement('canvas');
        copy.width = canvas.width; copy.height = canvas.height;
        const context = copy.getContext('2d');
        context.drawImage(canvas, 0, 0);
        const scale = canvas.width / canvas.clientWidth;
        const probe = (rect, exclude) => {
          const [pw, ps, pe, pn] = rect;
          let onScreen = 0, content = 0;
          for (let i = 1; i <= 9; i++) {
            for (let j = 1; j <= 9; j++) {
              const lon = pw + (pe - pw) * (0.1 + 0.8 * i / 10), lat = ps + (pn - ps) * (0.1 + 0.8 * j / 10);
              if (exclude && lon >= exclude[0] && lon <= exclude[2] && lat >= exclude[1] && lat <= exclude[3]) continue;
              const p = C.SceneTransforms.worldToWindowCoordinates(scene, C.Cartesian3.fromDegrees(lon, lat));
              if (!p || p.x < 0 || p.y < 0 || p.x >= canvas.clientWidth || p.y >= canvas.clientHeight) continue;
              const [r, g, b] = context.getImageData(Math.floor(p.x * scale), Math.floor(p.y * scale), 1, 1).data;
              onScreen++;
              if (Math.max(Math.abs(r - base.red * 255), Math.abs(g - base.green * 255),
                  Math.abs(b - base.blue * 255)) > 8) content++;
            }
          }
          return {onScreen, content};
        };
        if (!hole) return {opaque: probe([w, s, e, n])};
        const pad = [(hole[2] - hole[0]) * 0.1, (hole[3] - hole[1]) * 0.1];
        return {opaque: probe([w, s, e, n], [hole[0] - pad[0], hole[1] - pad[1], hole[2] + pad[0], hole[3] + pad[1]]),
          hole: probe(hole)};
      };
      const shown = await sample();
      window.orthophoto.show = false;
      const hidden = await sample();
      window.orthophoto.show = true;
      return {version: C.VERSION, loaded: window.loaded === true, failures: window.failures.slice(),
        bounds: metadata.bounds, shown, hidden};
    }, hole);
    console.log(JSON.stringify({...result, imageryTiles, browserErrors: errors,
      failedRequests: failed, externalRequests: external}, null, 2));
    assert.equal(result.version, '1.146.0');
    assert.deepEqual(result.failures, []);
    assert.deepEqual(errors, []);
    assert.equal(result.loaded, true);
    assert.deepEqual(failed, []);
    assert.deepEqual(external, []);
    assert.ok(imageryTiles > 0, 'no imagery tiles were served');
    const probes = [[result.shown.opaque, 'content'], [result.shown.hole, 'background'],
      [result.hidden.opaque, 'background'], [result.hidden.hole, 'background']].filter(([probe]) => probe);
    for (const [probe, expect] of probes) {
      assert.ok(probe.onScreen >= 20, JSON.stringify(probe));
      if (expect === 'content') assert.ok(probe.content >= 0.95 * probe.onScreen, `imagery not rendered: ${JSON.stringify(probe)}`);
      else assert.ok(probe.content <= 0.05 * probe.onScreen, `expected bare globe: ${JSON.stringify(probe)}`);
    }
  } finally {
    await browser.close();
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
