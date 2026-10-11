// Load one served output in pinned Cesium and report runtime behaviour as JSON.
// Usage: node cesium.cjs URL SPEC_JSON SCREENSHOT_PNG   (NODE_PATH exposes Playwright)
// SPEC: {kind:'tileset', url} or {kind:'raster', url, probes:[{name, rect:[w,s,e,n], expect}],
// views:[{name, rect, probes}]} (extra raster cameras after the overview), plus timeoutMs and
// an optional raster insetDegrees. Only 127.0.0.1 is reachable; other requests are aborted and reported.
const {chromium} = require('playwright');
const fs = require('node:fs');

async function run(spec) {
  const C = Cesium, raster = spec.kind === 'raster', failures = [];
  const viewer = new C.Viewer('view', {globe: raster ? undefined : false, baseLayer: false, skyBox: false,
    skyAtmosphere: false, geocoder: false, baseLayerPicker: false, animation: false, timeline: false,
    sceneModePicker: false, navigationHelpButton: false, homeButton: false, fullscreenButton: false,
    selectionIndicator: false, infoBox: false, contextOptions: {webgl: {preserveDrawingBuffer: true}}});
  const scene = viewer.scene;
  scene.postProcessStages.fxaa.enabled = false;
  scene.backgroundColor = C.Color.BLACK;
  scene.renderError.addEventListener((unused, error) => failures.push(String(error)));
  if (raster) {
    scene.globe.baseColor = C.Color.MAGENTA;
    scene.globe.showGroundAtmosphere = false;
    scene.fog.enabled = false;
    scene.globe.enableLighting = false;
  }
  // Ready when the condition holds on several consecutive completed frames.
  const settle = (ready, timeoutMs) => new Promise(resolve => {
    let stable = 0, remove = () => {};
    const timer = setTimeout(() => { remove(); resolve(false); }, timeoutMs);
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
  const report = {version: C.VERSION, canvas: [scene.canvas.width, scene.canvas.height]};
  if (!raster) {
    const start = performance.now();
    let tiles;
    try { tiles = await C.Cesium3DTileset.fromUrl(spec.url, {maximumScreenSpaceError: 16}); }
    catch (error) { return {...report, loaded: false, failures: [String(error)]}; }
    let loadedTiles = 0, allTilesLoaded = 0;
    tiles.tileLoad.addEventListener(() => loadedTiles++);
    tiles.tileFailed.addEventListener(error => failures.push(String(error.message || error)));
    tiles.allTilesLoaded.addEventListener(() => allTilesLoaded++);
    scene.primitives.add(tiles);
    const sphere = tiles.boundingSphere;
    viewer.camera.viewBoundingSphere(sphere, new C.HeadingPitchRange(-.8, -.65, Math.max(sphere.radius * 3, 5)));
    viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
    const loaded = await settle(() => tiles.tilesLoaded && allTilesLoaded > 0, spec.timeoutMs);
    return Object.assign(report, {loaded: loaded && !failures.length, secondsToLoaded: (performance.now() - start) / 1000,
      tilesLoadedEvents: loadedTiles, selectedTiles: (tiles._selectedTiles || []).length,
      tilesetMemoryBytes: tiles.totalMemoryUsageInBytes, usedJSHeapSize: heap(), failures,
      rootBoundingSphereRadius: sphere.radius});
  }
  const metadata = await (await fetch(spec.url + 'tilejson.json')).json();
  // Optional inset (degrees) works around Cesium's skeleton error on tile-aligned bounds.
  const [w, s, e, n] = metadata.bounds, d = spec.insetDegrees || 0;
  const provider = new C.UrlTemplateImageryProvider({url: new URL('tiles/', new URL(spec.url, location.href)).href + '{z}/{x}/{y}.png',
    minimumLevel: metadata.minzoom, maximumLevel: metadata.maxzoom, rectangle: C.Rectangle.fromDegrees(w + d, s + d, e - d, n - d)});
  let queued = 1, imageryErrors = 0, layer;
  provider.errorEvent.addEventListener(error => { imageryErrors++; failures.push(String(error.error || error.message || error)); });
  scene.globe.tileLoadProgressEvent.addEventListener(length => { queued = length; });
  report.views = [];
  // Each view: a bare-globe reference frame, then the same camera with the imagery shown.
  for (const view of [{name: 'overview', rect: metadata.bounds, probes: spec.probes}, ...(spec.views || [])]) {
    viewer.camera.setView({destination: C.Rectangle.fromDegrees(...view.rect)});
    if (layer) layer.show = false;
    await settle(() => scene.globe.tilesLoaded, spec.timeoutMs);
    const background = pixels(), start = performance.now();
    if (layer) layer.show = true; else layer = viewer.imageryLayers.addImageryProvider(provider);
    const loaded = await settle(() => scene.globe.tilesLoaded && queued === 0, spec.timeoutMs);
    const seconds = (performance.now() - start) / 1000, image = pixels();
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
    report.views.push({name: view.name, loaded, secondsToLoaded: seconds,
      changedPixelFraction: changed / (image.width * image.height), probes, png: canvas.toDataURL('image/png')});
  }
  return Object.assign(report, {loaded: report.views.every(v => v.loaded) && !failures.length,
    secondsToLoaded: report.views[0].secondsToLoaded, imageryErrors, probes: report.views.flatMap(v => v.probes),
    usedJSHeapSize: heap(), failures});
}

(async () => {
  const [url, specText, screenshot] = process.argv.slice(2), spec = JSON.parse(specText);
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM, headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader']});
  const console_ = {errors: [], warnings: []}, failedRequests = [], externalRequests = [];
  try {
    const page = await browser.newPage({viewport: {width: 1024, height: 768}});
    page.on('pageerror', error => console_.errors.push(String(error)));
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
    await page.goto(url);
    await page.waitForFunction(() => window.Cesium);
    await page.addScriptTag({content: run.toString()});
    const result = await page.evaluate(async spec => {
      try { return await run(spec); } catch (error) { return {loaded: false, failures: ['threw: ' + String(error)]}; }
    }, spec);
    await page.screenshot({path: screenshot});
    for (const view of result.views || []) {  // canvas captures of each raster view
      fs.writeFileSync(screenshot.replace(/\.png$/, `-${view.name}.png`), Buffer.from(view.png.split(',')[1], 'base64'));
      delete view.png;
    }
    console.log(JSON.stringify({...result, browserVersion: browser.version(), console: console_, failedRequests, externalRequests}));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
