// Load one served output in pinned Cesium and report runtime behaviour as JSON.
// Usage: node cesium.cjs URL SPEC_JSON SCREENSHOT_PNG   (NODE_PATH exposes Playwright)
// SPEC: {kind:'tileset', url, maximumScreenSpaceError} or {kind:'raster', url, probes:[{name, rect:[w,s,e,n], expect}],
// views:[{name, rect, probes}]} (extra raster cameras after the overview), plus stepMs and an
// optional raster insetDegrees [w,s,e,n]. Every asynchronous step (page load, fromUrl, TileJSON
// fetch, each settle, the screenshot) is bounded by stepMs; cesium.py derives the case budget.
// Only 127.0.0.1 is reachable; other requests and every WebSocket are refused and reported.
const {chromium} = require('playwright');
const fs = require('node:fs');

async function run(spec) {
  const C = Cesium, raster = spec.kind === 'raster', failures = [];
  const describe = error => (error && error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000);
  const bounded = (promise, label) => Promise.race([promise, new Promise((unused, reject) =>
    setTimeout(() => reject(new Error(`${label} exceeded ${spec.stepMs} ms`)), spec.stepMs))]);
  const viewer = new C.Viewer('view', {globe: raster ? undefined : false, baseLayer: false, skyBox: false,
    skyAtmosphere: false, geocoder: false, baseLayerPicker: false, animation: false, timeline: false,
    sceneModePicker: false, navigationHelpButton: false, homeButton: false, fullscreenButton: false,
    selectionIndicator: false, infoBox: false, contextOptions: {webgl: {preserveDrawingBuffer: true}}});
  const scene = viewer.scene;
  scene.postProcessStages.fxaa.enabled = false;
  scene.backgroundColor = C.Color.BLACK;
  scene.renderError.addEventListener((unused, error) => failures.push('renderError: ' + describe(error)));
  if (raster) {
    scene.globe.baseColor = C.Color.MAGENTA;
    scene.globe.showGroundAtmosphere = false;
    scene.fog.enabled = false;
    scene.globe.enableLighting = false;
  }
  // Ready when the condition holds on five consecutive completed frames; false after stepMs.
  const settle = ready => new Promise(resolve => {
    let stable = 0, remove = () => {};
    const timer = setTimeout(() => { remove(); resolve(false); }, spec.stepMs);
    remove = scene.postRender.addEventListener(() => {
      stable = ready() ? stable + 1 : 0;
      if (stable >= 5 || failures.length) { remove(); clearTimeout(timer); resolve(stable >= 5); }
      else scene.requestRender();
    });
    scene.requestRender();
  });
  const pixels = () => {
    const canvas = document.createElement('canvas');
    canvas.width = scene.canvas.width; canvas.height = scene.canvas.height;
    const context = canvas.getContext('2d');
    context.drawImage(scene.canvas, 0, 0);
    return context.getImageData(0, 0, canvas.width, canvas.height);
  };
  const heap = () => performance.memory ? performance.memory.usedJSHeapSize : null;
  const report = {version: C.VERSION, canvas: [scene.canvas.width, scene.canvas.height], failures};
  if (!raster) {
    const start = performance.now();
    let tiles;
    try { tiles = await bounded(C.Cesium3DTileset.fromUrl(spec.url, {maximumScreenSpaceError: spec.maximumScreenSpaceError ?? 16}), 'fromUrl'); }
    catch (error) { failures.push('fromUrl: ' + describe(error)); return {...report, loaded: false}; }
    let loadedTiles = 0, allTilesLoaded = 0;
    tiles.tileLoad.addEventListener(() => loadedTiles++);
    tiles.tileFailed.addEventListener(error => failures.push('tileFailed: ' + describe(error.message || error)));
    tiles.allTilesLoaded.addEventListener(() => allTilesLoaded++);
    scene.primitives.add(tiles);
    const sphere = tiles.boundingSphere;
    viewer.camera.viewBoundingSphere(sphere, new C.HeadingPitchRange(-.8, -.65, Math.max(sphere.radius * 3, 5)));
    viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
    const loaded = await settle(() => tiles.tilesLoaded && allTilesLoaded > 0);
    return Object.assign(report, {loaded: loaded && !failures.length, secondsToLoaded: (performance.now() - start) / 1000,
      tilesLoadedEvents: loadedTiles, selectedTiles: (tiles._selectedTiles || []).length,
      tilesetMemoryBytes: tiles.totalMemoryUsageInBytes, usedJSHeapSize: heap(), rootBoundingSphereRadius: sphere.radius});
  }
  const metadata = await bounded(fetch(spec.url + 'tilejson.json').then(response => response.json()), 'TileJSON fetch');
  // Optional per-edge inset (degrees): a labelled workaround, not what preview did before T1.
  const [w, s, e, n] = metadata.bounds, [dw, ds, de, dn] = spec.insetDegrees || [0, 0, 0, 0];
  const rectangle = C.Rectangle.fromDegrees(w + dw, s + ds, e - de, n - dn);
  report.providerRectangleDegrees = [w + dw, s + ds, e - de, n - dn];
  let queued = 1, imageryErrors = 0;
  scene.globe.tileLoadProgressEvent.addEventListener(length => { queued = length; });
  // As preview does: the layer exists from startup, before any globe tile is loaded.
  const start = performance.now();
  const provider = new C.UrlTemplateImageryProvider({url: new URL('tiles/', new URL(spec.url, location.href)).href + '{z}/{x}/{y}.png',
    minimumLevel: metadata.minzoom, maximumLevel: metadata.maxzoom, rectangle});
  provider.errorEvent.addEventListener(error => { imageryErrors++; failures.push('imagery: ' + describe(error.error || error.message || error)); });
  const layer = viewer.imageryLayers.addImageryProvider(provider);
  report.views = [];
  // Each view: settle with imagery, then the same camera with the layer hidden as the bare-globe reference.
  for (const view of [{name: 'overview', rect: metadata.bounds, probes: spec.probes}, ...(spec.views || [])]) {
    const viewStart = report.views.length ? performance.now() : start;
    viewer.camera.setView({destination: C.Rectangle.fromDegrees(...view.rect)});
    layer.show = true;
    const loaded = await settle(() => scene.globe.tilesLoaded && queued === 0);
    const seconds = (performance.now() - viewStart) / 1000, image = pixels();
    layer.show = false;
    const backgroundSettled = await settle(() => scene.globe.tilesLoaded);
    const background = pixels();
    if (failures.length) { report.views.push({name: view.name, loaded: false, backgroundSettled}); break; }
    const scale = image.width / scene.canvas.clientWidth;
    const differs = index => Math.max(...[0, 1, 2].map(c => Math.abs(image.data[index + c] - background.data[index + c]))) > 8;
    let changed = 0;
    for (let index = 0; index < image.data.length; index += 4) if (differs(index)) changed++;
    const probes = (view.probes || []).map(probe => {
      const [w, s, e, n] = probe.rect, samples = [];
      for (let i = 0; i < 15; i++) for (let j = 0; j < 15; j++) {
        const lon = w + (e - w) * (.1 + .8 * i / 14), lat = s + (n - s) * (.1 + .8 * j / 14), x = probe.exclude;
        if (x && lon > x[0] && lon < x[2] && lat > x[1] && lat < x[3]) continue;
        const screen = C.SceneTransforms.worldToWindowCoordinates(scene, C.Cartesian3.fromDegrees(lon, lat));
        if (!screen) continue;
        const px = Math.floor(screen.x * scale), py = Math.floor(screen.y * scale);
        if (px < 0 || py < 0 || px >= image.width || py >= image.height) continue;
        samples.push(differs((py * image.width + px) * 4));
      }
      const content = samples.filter(Boolean).length / Math.max(samples.length, 1);
      const pass = samples.length >= 20 && (probe.expect === 'background' ? content <= .05 : content >= .95);
      return {name: `${view.name}/${probe.name}`, expect: probe.expect, samples: samples.length, contentFraction: content, pass};
    });
    const canvas = document.createElement('canvas');
    canvas.width = image.width; canvas.height = image.height; canvas.getContext('2d').putImageData(image, 0, 0);
    report.views.push({name: view.name, loaded, backgroundSettled, secondsToLoaded: seconds,
      changedPixelFraction: changed / (image.width * image.height), probes, png: canvas.toDataURL('image/png')});
  }
  layer.show = true;  // settled again so the page screenshot shows the last view with imagery
  const restored = await settle(() => scene.globe.tilesLoaded && queued === 0);
  return Object.assign(report, {loaded: report.views.every(v => v.loaded && v.backgroundSettled) && restored && !failures.length,
    secondsToLoaded: report.views[0].secondsToLoaded, imageryErrors, probes: report.views.flatMap(v => v.probes || []),
    usedJSHeapSize: heap()});
}

(async () => {
  const [url, specText, screenshot] = process.argv.slice(2), spec = JSON.parse(specText);
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM, headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']});
  const console_ = {errors: [], warnings: []}, failedRequests = [], externalRequests = [];
  try {
    const page = await browser.newPage({viewport: {width: 1024, height: 768}});
    page.setDefaultTimeout(spec.stepMs);
    page.on('pageerror', error => console_.errors.push(error.stack ? `${error}\n${error.stack}` : String(error)));
    page.on('console', message => {
      if (message.type() === 'error') console_.errors.push(message.text());
      else if (message.type() === 'warning') console_.warnings.push(message.text());
    });
    page.on('requestfailed', request => failedRequests.push({url: request.url(), error: request.failure()?.errorText}));
    page.on('response', response => {
      if (response.status() >= 400) failedRequests.push({url: response.url(), status: response.status()});
    });
    await page.route('**/*', route => {
      const target = new URL(route.request().url());
      if (target.hostname !== '127.0.0.1' && target.protocol !== 'data:' && target.protocol !== 'blob:') {
        externalRequests.push(target.href); return route.abort();
      }
      return route.continue();
    });
    await page.routeWebSocket(/.*/, socket => { externalRequests.push(socket.url()); socket.close(); });
    await page.goto(url);
    await page.waitForFunction(() => window.Cesium);
    await page.addScriptTag({content: run.toString()});
    const result = await page.evaluate(async spec => {
      try { return await run(spec); }
      catch (error) { return {loaded: false, failures: ['threw: ' + (error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000)]}; }
    }, spec);
    try { await page.screenshot({path: screenshot}); }  // a bounded step; its failure keeps the result
    catch (error) { result.loaded = false; (result.failures = result.failures || []).push('screenshot: ' + String(error).slice(0, 2000)); }
    for (const view of result.views || []) {  // canvas captures of each raster view
      if (!view.png) continue;
      fs.writeFileSync(screenshot.replace(/\.png$/, `-${view.name}.png`), Buffer.from(view.png.split(',')[1], 'base64'));
      delete view.png;
    }
    console.log(JSON.stringify({...result, browserVersion: browser.version(), console: console_, failedRequests, externalRequests}));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
