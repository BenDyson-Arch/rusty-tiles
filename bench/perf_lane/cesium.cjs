// Load one served output in pinned Cesium and report runtime behaviour as JSON.
// Usage: node cesium.cjs URL SPEC_JSON SCREENSHOT_PNG   (NODE_PATH exposes Playwright)
//        node cesium.cjs --renderer SPEC_JSON            (preflight: WebGL renderer, browser and tab facts only)
// SPEC: {kind:'tileset', url, maximumScreenSpaceError} or {kind:'raster', url, probes:[{name, rect:[w,s,e,n], expect}],
// views:[{name, rect, probes}]} (extra raster cameras after the overview), plus stepMs, settleMs, browser,
// an optional scheduler ({throttleRequests, maximumRequestsPerServer, maximumRequests}: a viewer setting,
// diagnostic only) and an optional raster insetDegrees [w,s,e,n]. Every asynchronous step (page load,
// fromUrl, TileJSON fetch, each settle, the screenshot) is bounded by stepMs; cesium.py derives the budget.
// spec.browser is {mode:'cdp', cdpUrl} (default: a running Chromium; this script opens its own context
// and closes only that, never the browser) or {mode:'launch', gl, executablePath} (a private headless one).
// Only 127.0.0.1 is reachable; other requests and every WebSocket are refused and reported.
const fs = require('node:fs');

// Chromium flags for --launch; the CDP browser keeps whatever its owner started it with.
const GL_ARGS = {
  gl: ['--use-angle=gl', '--ignore-gpu-blocklist', '--enable-gpu'],
  'gl-egl': ['--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu'],
  vulkan: ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist', '--enable-gpu'],
  swiftshader: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
};
const VIEWPORT = {width: 1024, height: 768};
const FACTS = `(() => { ${webglInfo}
 return (${pageFacts})(); })()`;

// Finish rule: done once ready() has held on every completed frame for windowMs of wall time.
// observe() is called once per frame; it returns {at, frame} (when the ready run began) once the run
// has lasted windowMs, else null. A frame where ready() is false restarts the window.
function finishTracker(windowMs) {
  let since = null, frames = 0;
  return {
    observe(ready, now) {
      frames++;
      if (!ready) { since = null; return null; }
      if (since === null) since = {at: now, frame: frames};
      return now - since.at >= windowMs ? since : null;
    },
  };
}

// WebGL driver strings from a scratch canvas in the page (same GPU process and ANGLE backend as Cesium's).
function webglInfo() {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
  if (!gl) return {renderer: null, vendor: null, webgl: null};
  const debug = gl.getExtension('WEBGL_debug_renderer_info');
  return {renderer: gl.getParameter(debug ? debug.UNMASKED_RENDERER_WEBGL : gl.RENDERER),
    vendor: gl.getParameter(debug ? debug.UNMASKED_VENDOR_WEBGL : gl.VENDOR),
    webgl: gl.getParameter(gl.VERSION), unmasked: Boolean(debug)};
}

function pageFacts() {
  return {visibility: document.visibilityState, hasFocus: document.hasFocus(), viewport: [innerWidth, innerHeight],
    window: [outerWidth, outerHeight], devicePixelRatio, ...webglInfo()};
}

async function run(spec) {
  const C = Cesium, raster = spec.kind === 'raster', failures = [];
  const describe = error => (error && error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000);
  const bounded = (promise, label) => Promise.race([promise, new Promise((unused, reject) =>
    setTimeout(() => reject(new Error(`${label} exceeded ${spec.stepMs} ms`)), spec.stepMs))]);
  // A background tab throttles requestAnimationFrame, so every frame-paced number would be wrong.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') failures.push(`tab became ${document.visibilityState} during the run`);
  });
  // In-flight XHR/fetch count, part of "no requests pending".
  let inflight = 0;
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function (...args) {
    inflight++; this.addEventListener('loadend', () => inflight--, {once: true});
    return send.apply(this, args);
  };
  const fetch_ = window.fetch;
  window.fetch = (...args) => { inflight++; return fetch_(...args).finally(() => inflight--); };
  if (spec.scheduler) Object.assign(C.RequestScheduler, spec.scheduler);
  const viewer = new C.Viewer('view', {globe: raster ? undefined : false, baseLayer: false, skyBox: false,
    skyAtmosphere: false, geocoder: false, baseLayerPicker: false, animation: false, timeline: false,
    sceneModePicker: false, navigationHelpButton: false, homeButton: false, fullscreenButton: false,
    selectionIndicator: false, infoBox: false, contextOptions: {webgl: {preserveDrawingBuffer: true}}});
  const scene = viewer.scene, scheduler = C.RequestScheduler;
  scene.postProcessStages.fxaa.enabled = false;
  scene.backgroundColor = C.Color.BLACK;
  scene.renderError.addEventListener((unused, error) => failures.push('renderError: ' + describe(error)));
  if (raster) {
    scene.globe.baseColor = C.Color.MAGENTA;
    scene.globe.showGroundAtmosphere = false;
    scene.fog.enabled = false;
    scene.globe.enableLighting = false;
  }
  let peakActiveRequests = 0;
  scene.postRender.addEventListener(() => {
    peakActiveRequests = Math.max(peakActiveRequests, scheduler.statistics.numberOfActiveRequests);
  });
  const idle = () => inflight === 0 && scheduler.statistics.numberOfActiveRequests === 0;
  // Resolves {loaded, at, frames, confirmedAt} under finishTracker(settleMs); {loaded: false} after stepMs.
  const settle = ready => new Promise(resolve => {
    const tracker = finishTracker(spec.settleMs);
    let remove = () => {};
    const timer = setTimeout(() => { remove(); resolve({loaded: false}); }, spec.stepMs);
    remove = scene.postRender.addEventListener(() => {
      const now = performance.now(), done = tracker.observe(ready() && idle(), now);
      if (done || failures.length) {
        remove(); clearTimeout(timer);
        resolve(done && !failures.length ? {loaded: true, at: done.at, frames: done.frame, confirmedAt: now} : {loaded: false});
      } else scene.requestRender();
    });
    scene.requestRender();
  });
  const seconds = (from, to) => (to == null ? null : (to - from) / 1000);
  const pixels = () => {
    const canvas = document.createElement('canvas');
    canvas.width = scene.canvas.width; canvas.height = scene.canvas.height;
    const context = canvas.getContext('2d');
    context.drawImage(scene.canvas, 0, 0);
    return context.getImageData(0, 0, canvas.width, canvas.height);
  };
  const heap = () => performance.memory ? performance.memory.usedJSHeapSize : null;
  const report = {version: C.VERSION, canvas: [scene.canvas.width, scene.canvas.height], failures,
    scheduler: {throttleRequests: scheduler.throttleRequests, maximumRequests: scheduler.maximumRequests,
      maximumRequestsPerServer: scheduler.maximumRequestsPerServer}};
  if (!raster) {
    const start = performance.now();
    let tiles;
    try { tiles = await bounded(C.Cesium3DTileset.fromUrl(spec.url, {maximumScreenSpaceError: spec.maximumScreenSpaceError ?? 16}), 'fromUrl'); }
    catch (error) { failures.push('fromUrl: ' + describe(error)); return {...report, loaded: false}; }
    let loadedTiles = 0, allTilesLoaded = 0, firstTile = null;
    tiles.tileLoad.addEventListener(() => { loadedTiles++; if (firstTile === null) firstTile = performance.now(); });
    tiles.tileFailed.addEventListener(error => failures.push('tileFailed: ' + describe(error.message || error)));
    tiles.allTilesLoaded.addEventListener(() => allTilesLoaded++);
    scene.primitives.add(tiles);
    const sphere = tiles.boundingSphere;
    viewer.camera.viewBoundingSphere(sphere, new C.HeadingPitchRange(-.8, -.65, Math.max(sphere.radius * 3, 5)));
    viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
    const done = await settle(() => {
      const statistics = tiles.statistics;
      return tiles.tilesLoaded && allTilesLoaded > 0 && statistics.numberOfPendingRequests === 0
        && statistics.numberOfTilesProcessing === 0;
    });
    return Object.assign(report, {loaded: done.loaded && !failures.length, secondsToLoaded: seconds(start, done.at),
      secondsToSettled: seconds(start, done.confirmedAt), framesToLoaded: done.frames ?? null,
      secondsToFirstTile: seconds(start, firstTile), peakActiveRequests,
      tilesLoadedEvents: loadedTiles, selectedTiles: (tiles._selectedTiles || []).length,
      tilesetMemoryBytes: tiles.totalMemoryUsageInBytes, usedJSHeapSize: heap(), rootBoundingSphereRadius: sphere.radius});
  }
  const metadata = await bounded(fetch(spec.url + 'tilejson.json').then(response => response.json()), 'TileJSON fetch');
  // Optional per-edge inset (degrees): a labelled workaround, not what preview did before T1.
  const [w, s, e, n] = metadata.bounds, [dw, ds, de, dn] = spec.insetDegrees || [0, 0, 0, 0];
  const rectangle = C.Rectangle.fromDegrees(w + dw, s + ds, e - de, n - dn);
  report.providerRectangleDegrees = [w + dw, s + ds, e - de, n - dn];
  let queued = 1, imageryErrors = 0, firstTile = null;
  scene.globe.tileLoadProgressEvent.addEventListener(length => { queued = length; });
  // As preview does: the layer exists from startup, before any globe tile is loaded.
  const start = performance.now();
  const provider = new C.UrlTemplateImageryProvider({url: new URL('tiles/', new URL(spec.url, location.href)).href + '{z}/{x}/{y}.png',
    minimumLevel: metadata.minzoom, maximumLevel: metadata.maxzoom, rectangle});
  const requestImage = provider.requestImage.bind(provider);
  provider.requestImage = (...args) => {  // first imagery tile received, for secondsToFirstTile
    const promise = requestImage(...args);
    if (promise) promise.then(() => { if (firstTile === null) firstTile = performance.now(); }, () => {});
    return promise;
  };
  provider.errorEvent.addEventListener(error => { imageryErrors++; failures.push('imagery: ' + describe(error.error || error.message || error)); });
  const layer = viewer.imageryLayers.addImageryProvider(provider);
  report.views = [];
  // Each view: settle with imagery, then the same camera with the layer hidden as the bare-globe reference.
  for (const view of [{name: 'overview', rect: metadata.bounds, probes: spec.probes}, ...(spec.views || [])]) {
    const viewStart = report.views.length ? performance.now() : start;
    viewer.camera.setView({destination: C.Rectangle.fromDegrees(...view.rect)});
    layer.show = true;
    const done = await settle(() => scene.globe.tilesLoaded && queued === 0);
    const image = pixels();
    layer.show = false;
    const backgroundSettled = (await settle(() => scene.globe.tilesLoaded)).loaded;
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
    report.views.push({name: view.name, loaded: done.loaded, backgroundSettled, secondsToLoaded: seconds(viewStart, done.at),
      secondsToSettled: seconds(viewStart, done.confirmedAt), framesToLoaded: done.frames ?? null,
      changedPixelFraction: changed / (image.width * image.height), probes, png: canvas.toDataURL('image/png')});
  }
  layer.show = true;  // settled again so the page screenshot shows the last view with imagery
  const restored = (await settle(() => scene.globe.tilesLoaded && queued === 0)).loaded;
  const overview = report.views[0] || {};
  return Object.assign(report, {loaded: report.views.every(v => v.loaded && v.backgroundSettled) && restored && !failures.length,
    secondsToLoaded: overview.secondsToLoaded ?? null, secondsToSettled: overview.secondsToSettled ?? null,
    framesToLoaded: overview.framesToLoaded ?? null, secondsToFirstTile: seconds(start, firstTile), peakActiveRequests,
    imageryErrors, probes: report.views.flatMap(v => v.probes || []), usedJSHeapSize: heap()});
}

// A fresh context per run: an attached CDP browser is only ever disconnected from, never closed.
async function openContext(spec) {
  const {chromium} = require('playwright');
  const options = spec.browser || {mode: 'cdp', cdpUrl: 'http://localhost:9222'};
  if (options.mode === 'cdp') {
    let browser;
    try { browser = await chromium.connectOverCDP(options.cdpUrl, {timeout: 15000}); }
    catch (error) {
      throw new Error(`no Chromium DevTools endpoint at ${options.cdpUrl} (${String(error.message || error).split('\n')[0]}); `
        + 'start Chromium with --remote-debugging-port, set --cdp-url, or pass --launch');
    }
    const context = await browser.newContext({viewport: VIEWPORT});
    return {browser, context, mode: 'cdp', close: () => context.close()};
  }
  const browser = await chromium.launch({executablePath: options.executablePath, headless: true,
    args: ['--no-sandbox', ...GL_ARGS[options.gl || 'gl'], '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']});
  const context = await browser.newContext({viewport: VIEWPORT});
  return {browser, context, mode: 'launch', close: async () => { await context.close(); await browser.close(); }};
}

// New tab in our context, cache disabled over CDP (cold runs), brought to the front and checked visible.
async function openPage(opened) {
  const page = await opened.context.newPage();
  const session = await opened.context.newCDPSession(page);
  await session.send('Network.enable');
  await session.send('Network.setCacheDisabled', {cacheDisabled: true});
  await page.bringToFront();
  return page;
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv[0] === '--renderer') {
    const opened = await openContext({browser: JSON.parse(argv[1])});
    try {
      const page = await openPage(opened);
      const facts = await page.evaluate(FACTS);
      console.log(JSON.stringify({...facts, browserVersion: opened.browser.version(), mode: opened.mode, cacheDisabled: true}));
    } finally { await opened.close(); }
    return;
  }
  const [url, specText, screenshot] = argv, spec = JSON.parse(specText);
  const console_ = {errors: [], warnings: []}, failedRequests = [], externalRequests = [];
  const opened = await openContext(spec);
  try {
    const page = await openPage(opened);
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
    // A fresh window in an attached browser occasionally aborts its first navigation (net::ERR_ABORTED
    // before any of our requests); retry that, record it, and drop its failed document request.
    let navigationRetries = 0;
    for (;;) {
      try { await page.goto(url); break; }
      catch (error) {
        if (navigationRetries >= 2 || !/ERR_ABORTED/.test(String(error))) throw error;
        navigationRetries++;
        for (let i = failedRequests.length - 1; i >= 0; i--) if (failedRequests[i].url === url) failedRequests.splice(i, 1);
      }
    }
    await page.waitForFunction(() => window.Cesium);
    await page.bringToFront();
    const before = await page.evaluate(FACTS);
    let result;
    if (before.visibility !== 'visible') {
      result = {loaded: false, failures: [`tab is ${before.visibility} after bringToFront; frame-paced timings would be throttled`]};
    } else {
      await page.addScriptTag({content: [finishTracker, webglInfo, run].map(String).join('\n')});
      result = await page.evaluate(async spec => {
        try { return await run(spec); }
        catch (error) { return {loaded: false, failures: ['threw: ' + (error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000)]}; }
      }, spec);
    }
    try { await page.screenshot({path: screenshot}); }  // a bounded step; its failure keeps the result
    catch (error) { result.loaded = false; (result.failures = result.failures || []).push('screenshot: ' + String(error).slice(0, 2000)); }
    for (const view of result.views || []) {  // canvas captures of each raster view
      if (!view.png) continue;
      fs.writeFileSync(screenshot.replace(/\.png$/, `-${view.name}.png`), Buffer.from(view.png.split(',')[1], 'base64'));
      delete view.png;
    }
    const after = await page.evaluate(() => document.visibilityState);
    console.log(JSON.stringify({...result, browserVersion: opened.browser.version(), browserMode: opened.mode, cacheDisabled: true, navigationRetries,
      renderer: before.renderer, vendor: before.vendor, webgl: before.webgl, viewport: before.viewport, window: before.window,
      devicePixelRatio: before.devicePixelRatio, visibility: {before: before.visibility, after, hasFocus: before.hasFocus},
      console: console_, failedRequests, externalRequests}));
  } finally { await opened.close(); }
}

if (require.main === module) {
  // Exit explicitly: a CDP connection is left open (never browser.close() on someone else's browser).
  main().then(() => process.exit(0), error => { console.error(String(error.stack || error)); process.exit(1); });
}
module.exports = {finishTracker, GL_ARGS};
