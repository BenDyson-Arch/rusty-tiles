// Fly one deterministic camera path over one served tileset in pinned Cesium and record per-frame metrics.
// Usage: node flythrough.cjs URL SPEC_FILE SHOT_PREFIX   (NODE_PATH exposes Playwright)
// SPEC (written by flythrough.py): {url, steps:[{pose:[heading, pitch, range, east, north, up], waypoint?}],
// fps, holdMs, settleMs, stepMs, browser, maximumScreenSpaceError, tilesetOptions, scheduler?, cpuThrottlingRate,
// screenshots, trace, tracePath}. With trace, a Chrome trace (browser-wide, over CDP) spans the flythrough
// and is written gzipped to tracePath. Poses are relative to the root bounding sphere (degrees, multiples of its radius, in its
// east-north-up frame). The default render loop is off: each requestAnimationFrame sets the pose of the next
// step and the simulated clock (frame / fps), then renders once, so the camera replays identically on every
// run whatever the frame rate. A waypoint step renders at a fixed pose until full quality (cesium.cjs's
// finish rule) or holdMs. Prints one JSON result; waypoint captures go to SHOT_PREFIX-<waypoint>.png.
const fs = require('node:fs');
const zlib = require('node:zlib');
// Chrome trace categories for the untimed traced run: task boundaries, the DevTools timeline (JS, GC,
// decode), V8 and GPU command-buffer activity.
const TRACE_CATEGORIES = ['-*', 'toplevel', 'devtools.timeline', 'disabled-by-default-devtools.timeline', 'v8', 'v8.execute',
  'disabled-by-default-v8.gc', 'gpu'];
const {finishTracker, FACTS, webglInfo, openContext, openPage} = require('./cesium.cjs');

async function fly(spec) {
  const C = Cesium, failures = [];
  const describe = error => (error && error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000);
  const bounded = (promise, label) => Promise.race([promise, new Promise((unused, reject) =>
    setTimeout(() => reject(new Error(`${label} exceeded ${spec.stepMs} ms`)), spec.stepMs))]);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') failures.push(`tab became ${document.visibilityState} during the run`);
  });
  let inflight = 0;
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function (...args) {
    inflight++; this.addEventListener('loadend', () => inflight--, {once: true});
    return send.apply(this, args);
  };
  const fetch_ = window.fetch;
  window.fetch = (...args) => { inflight++; return fetch_(...args).finally(() => inflight--); };
  if (spec.scheduler) Object.assign(C.RequestScheduler, spec.scheduler);
  // Requests and bytes per frame: resource timing entries for the served data. After each render the
  // observer's buffer is drained synchronously (takeRecords), so an entry counts toward the first frame
  // rendered after its response completed; entries still buffered at the end go to the last frame.
  performance.setResourceTimingBufferSize(1000000);
  let resources = [];
  const keep = entries => { for (const entry of entries) if (entry.name.includes('/data/')) resources.push(entry); };
  const observer = new PerformanceObserver(list => keep(list.getEntries()));
  observer.observe({type: 'resource'});
  const viewer = new C.Viewer('view', {globe: false, baseLayer: false, skyBox: false, skyAtmosphere: false,
    geocoder: false, baseLayerPicker: false, animation: false, timeline: false, sceneModePicker: false,
    navigationHelpButton: false, homeButton: false, fullscreenButton: false, selectionIndicator: false, infoBox: false,
    // Only capture runs keep the drawing buffer (for toDataURL); timed runs render as a viewer would.
    useDefaultRenderLoop: false, contextOptions: {webgl: {preserveDrawingBuffer: Boolean(spec.screenshots)}}});
  const scene = viewer.scene, camera = viewer.camera, scheduler = C.RequestScheduler;
  viewer.clock.shouldAnimate = false;
  scene.postProcessStages.fxaa.enabled = false;
  scene.backgroundColor = C.Color.BLACK;
  scene.renderError.addEventListener((unused, error) => failures.push('renderError: ' + describe(error)));
  viewer.resize();
  // Draw calls and primitives: wrap the draw entry points of Cesium's own WebGL context.
  const gl = scene.context._gl, draw = {calls: 0, triangles: 0, other: 0};
  const primitives = (mode, count, instances) => {
    draw.calls++;
    if (mode === gl.TRIANGLES) draw.triangles += Math.floor(count / 3) * instances;
    else if (mode === gl.TRIANGLE_STRIP || mode === gl.TRIANGLE_FAN) draw.triangles += Math.max(count - 2, 0) * instances;
    else draw.other += count * instances;
  };
  const wrap = (name, pick) => {
    if (typeof gl[name] !== 'function') return;
    const original = gl[name].bind(gl);
    gl[name] = (...args) => { pick(args); return original(...args); };
  };
  wrap('drawArrays', a => primitives(a[0], a[2], 1));
  wrap('drawElements', a => primitives(a[0], a[1], 1));
  wrap('drawArraysInstanced', a => primitives(a[0], a[2], a[3]));
  wrap('drawElementsInstanced', a => primitives(a[0], a[1], a[4]));
  wrap('drawRangeElements', a => primitives(a[0], a[3], 1));
  // GPU time per frame where EXT_disjoint_timer_query_webgl2 is exposed; results arrive frames later.
  const timer = gl.getExtension && gl.getExtension('EXT_disjoint_timer_query_webgl2');
  const gpuPending = [];
  const columns = {t: [], cpu: [], gpu: [], hold: [], gap: [], visited: [], selected: [], commands: [], drawCalls: [],
    triangles: [], otherPrimitives: [], ready: [], pending: [], processing: [], geometryBytes: [], textureBytes: [],
    memoryBytes: [], loads: [], unloads: [], reloads: [], requests: [], requestBytes: [], selAdded: [], selRemoved: [],
    maxSse: [], memorySse: []};
  const start = performance.now();
  let tiles;
  try {
    tiles = await bounded(C.Cesium3DTileset.fromUrl(spec.url, {...(spec.tilesetOptions || {}),
      maximumScreenSpaceError: spec.maximumScreenSpaceError ?? 16}), 'fromUrl');
  } catch (error) { failures.push('fromUrl: ' + describe(error)); return {loaded: false, failures}; }
  const fromUrlSeconds = (performance.now() - start) / 1000;
  // Cache churn: content loads, evictions (tileUnload) and loads of a tile evicted before.
  const evicted = new WeakSet(), churn = {loads: 0, unloads: 0, reloads: 0};
  let allTilesLoaded = 0;
  tiles.tileLoad.addEventListener(tile => { churn.loads++; if (evicted.has(tile)) churn.reloads++; });
  tiles.tileUnload.addEventListener(tile => { churn.unloads++; evicted.add(tile); });
  tiles.tileFailed.addEventListener(error => failures.push('tileFailed: ' + describe(error.message || error)));
  tiles.allTilesLoaded.addEventListener(() => allTilesLoaded++);
  scene.primitives.add(tiles);
  const sphere = tiles.boundingSphere, radius = Math.max(sphere.radius, 1);
  // Georeferenced content gets its east-north-up frame; content near the earth's centre a plain translation.
  const frame = C.Cartesian3.magnitude(sphere.center) > 1e5 ? C.Transforms.eastNorthUpToFixedFrame(sphere.center)
    : C.Matrix4.fromTranslation(sphere.center);
  const setPose = ([heading, pitch, range, east, north, up]) => {
    const target = C.Matrix4.multiplyByPoint(frame, new C.Cartesian3(east * radius, north * radius, up * radius), new C.Cartesian3());
    camera.lookAt(target, new C.HeadingPitchRange(C.Math.toRadians(heading), C.Math.toRadians(pitch), Math.max(range * radius, 5)));
    camera.lookAtTransform(C.Matrix4.IDENTITY);
  };
  const ids = new WeakMap();
  let nextId = 0, previous = new Set(), frameIndex = 0, firstRender = null, gapNext = false;
  const epoch = C.JulianDate.fromIso8601('2026-01-01T12:00:00Z');
  const idle = () => inflight === 0 && scheduler.statistics.numberOfActiveRequests === 0;
  const fullQuality = () => {
    const s = tiles.statistics;
    return tiles.tilesLoaded && allTilesLoaded > 0 && s.numberOfPendingRequests === 0 && s.numberOfTilesProcessing === 0 && idle();
  };
  // The disjoint flag is read once per poll: when set, every query pending at that moment is unreliable
  // and is dropped (its frames keep gpu null); otherwise available results are read in order.
  let gpuDisjoint = 0;
  const pollGpu = () => {
    if (gl.getParameter(timer.GPU_DISJOINT_EXT)) {
      gpuDisjoint += gpuPending.length;
      for (const [query] of gpuPending) gl.deleteQuery(query);
      gpuPending.length = 0;
      return;
    }
    while (gpuPending.length) {
      const [query, index] = gpuPending[0];
      if (!gl.getQueryParameter(query, gl.QUERY_RESULT_AVAILABLE)) break;
      columns.gpu[index] = gl.getQueryParameter(query, gl.QUERY_RESULT) / 1e6;
      gl.deleteQuery(query); gpuPending.shift();
    }
  };
  // One presented frame at the current pose; returns whether the tileset is at full quality.
  const renderFrame = async holding => {
    const stamp = await new Promise(requestAnimationFrame);
    viewer.clock.currentTime = C.JulianDate.addSeconds(epoch, frameIndex / spec.fps, new C.JulianDate());
    draw.calls = draw.triangles = draw.other = 0;
    let query = null;
    if (timer) { query = gl.createQuery(); gl.beginQuery(timer.TIME_ELAPSED_EXT, query); }
    const before = performance.now();
    viewer.render();
    const cpu = performance.now() - before;
    if (query) { gl.endQuery(timer.TIME_ELAPSED_EXT); gpuPending.push([query, frameIndex]); }
    columns.gpu.push(null);
    if (timer) pollGpu();
    const s = tiles.statistics, selected = tiles._selectedTiles || [], current = new Set();
    let added = 0, maxSse = 0;
    for (const tile of selected) {
      if (!ids.has(tile)) ids.set(tile, nextId++);
      const id = ids.get(tile);
      current.add(id);
      if (!previous.has(id)) added++;
      if (tile._screenSpaceError > maxSse) maxSse = tile._screenSpaceError;
    }
    const removed = previous.size - (current.size - added);
    previous = current;
    keep(observer.takeRecords());
    const arrived = resources; resources = [];
    if (firstRender === null && s.numberOfCommands > 0) firstRender = performance.now();
    const ready = fullQuality();
    const push = (key, value) => columns[key].push(value);
    push('t', stamp); push('cpu', cpu); push('hold', holding ? 1 : 0); push('gap', gapNext ? 1 : 0);
    push('visited', s.visited); push('selected', s.selected); push('commands', s.numberOfCommands);
    push('drawCalls', draw.calls); push('triangles', draw.triangles); push('otherPrimitives', draw.other);
    push('ready', s.numberOfTilesWithContentReady); push('pending', s.numberOfPendingRequests);
    push('processing', s.numberOfTilesProcessing); push('geometryBytes', s.geometryByteLength);
    push('textureBytes', s.texturesByteLength);
    const memory = tiles.totalMemoryUsageInBytes;  // NaN when some content reports no byte length
    push('memoryBytes', Number.isFinite(memory) ? memory : null);
    push('loads', churn.loads); push('unloads', churn.unloads); push('reloads', churn.reloads);
    push('requests', arrived.length);
    push('requestBytes', arrived.reduce((sum, e) => sum + (e.encodedBodySize || e.transferSize || 0), 0));
    push('selAdded', added); push('selRemoved', removed); push('maxSse', maxSse);
    push('memorySse', tiles.memoryAdjustedScreenSpaceError ?? null);
    churn.loads = churn.unloads = churn.reloads = 0;  // events between frames count toward the next one
    gapNext = false; frameIndex++;
    return ready;
  };
  const waypoints = [];
  for (const step of spec.steps) {
    if (failures.length) break;
    setPose(step.pose);
    if (step.waypoint === undefined) { await renderFrame(false); continue; }
    const arrival = performance.now(), firstFrame = frameIndex, tracker = finishTracker(spec.settleMs);
    let done = null;
    while (!failures.length && performance.now() - arrival < spec.holdMs) {
      const ready = await renderFrame(true);
      done = tracker.observe(ready, performance.now());
      if (done) break;
    }
    const record = {name: step.waypoint, frame: firstFrame, reached: Boolean(done),
      fullQualitySeconds: done ? (done.at - arrival) / 1000 : null, framesToFullQuality: done ? done.frame : null,
      holdFrames: frameIndex - firstFrame};
    if (done && spec.screenshots) { record.png = scene.canvas.toDataURL('image/png'); gapNext = true; }
    waypoints.push(record);
  }
  // Let late timer queries resolve (a few frames at the last pose; not recorded).
  for (let i = 0; timer && gpuPending.length && i < 10; i++) { await new Promise(requestAnimationFrame); pollGpu(); }
  keep(observer.takeRecords());
  observer.disconnect();
  const last = columns.t.length - 1;
  if (last >= 0 && resources.length) {
    columns.requests[last] += resources.length;
    columns.requestBytes[last] += resources.reduce((sum, e) => sum + (e.encodedBodySize || e.transferSize || 0), 0);
  }
  return {loaded: !failures.length, failures, version: C.VERSION, canvas: [scene.canvas.width, scene.canvas.height],
    fromUrlSeconds, firstRenderSeconds: firstRender === null ? null : (firstRender - start) / 1000,
    totalSeconds: (performance.now() - start) / 1000, rootBoundingSphereRadius: sphere.radius,
    gpuTimer: timer ? 'EXT_disjoint_timer_query_webgl2' : null, gpuDisjointDropped: gpuDisjoint, heap: performance.memory ? performance.memory.usedJSHeapSize : null,
    scheduler: {throttleRequests: scheduler.throttleRequests, maximumRequests: scheduler.maximumRequests,
      maximumRequestsPerServer: scheduler.maximumRequestsPerServer},
    waypoints, frames: columns};
}

async function main() {
  const [url, specFile, shotPrefix] = process.argv.slice(2), spec = JSON.parse(fs.readFileSync(specFile, 'utf8'));
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
    page.on('response', response => { if (response.status() >= 400) failedRequests.push({url: response.url(), status: response.status()}); });
    await page.route('**/*', route => {
      const target = new URL(route.request().url());
      if (target.hostname !== '127.0.0.1' && target.protocol !== 'data:' && target.protocol !== 'blob:') {
        externalRequests.push(target.href); return route.abort();
      }
      return route.continue();
    });
    await page.routeWebSocket(/.*/, socket => { externalRequests.push(socket.url()); socket.close(); });
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
    // CPU throttling applies from here, after Cesium.js itself has loaded: the flythrough is what is constrained.
    const rate = spec.cpuThrottlingRate || 1;
    if (rate !== 1) await (await opened.context.newCDPSession(page)).send('Emulation.setCPUThrottlingRate', {rate});
    let result;
    if (before.visibility !== 'visible') {
      result = {loaded: false, failures: [`tab is ${before.visibility} after bringToFront; frame-paced timings would be throttled`]};
    } else {
      await page.addScriptTag({content: [finishTracker, webglInfo, fly].map(String).join('\n')});
      let tracing = false, traceError = null;
      if (spec.trace) {
        try { await opened.browser.startTracing(page, {categories: TRACE_CATEGORIES}); tracing = true; }
        catch (error) { traceError = 'startTracing: ' + String(error).slice(0, 500); }
      }
      // fly() is bounded by its own steps; the driver's budget in flythrough.py bounds the whole run.
      result = await page.evaluate(async spec => {
        try { return await fly(spec); }
        catch (error) { return {loaded: false, failures: ['threw: ' + (error.stack ? `${error}\n${error.stack}` : String(error)).slice(0, 4000)]}; }
      }, spec);
      if (tracing) {
        try { fs.writeFileSync(spec.tracePath, zlib.gzipSync(await opened.browser.stopTracing())); }
        catch (error) { traceError = 'stopTracing: ' + String(error).slice(0, 500); }
      }
      if (traceError) result.traceError = traceError;
    }
    for (const waypoint of result.waypoints || []) {
      if (!waypoint.png) continue;
      waypoint.screenshot = `${shotPrefix}-${waypoint.name}.png`;
      fs.writeFileSync(waypoint.screenshot, Buffer.from(waypoint.png.split(',')[1], 'base64'));
      delete waypoint.png;
    }
    const after = await page.evaluate(() => document.visibilityState);
    console.log(JSON.stringify({...result, browserVersion: opened.browser.version(), browserMode: opened.mode, cacheDisabled: true,
      navigationRetries, cpuThrottlingRate: rate, renderer: before.renderer, vendor: before.vendor, webgl: before.webgl,
      viewport: before.viewport, devicePixelRatio: before.devicePixelRatio,
      visibility: {before: before.visibility, after, hasFocus: before.hasFocus}, console: console_, failedRequests, externalRequests}));
  } finally { await opened.close(); }
}

if (require.main === module) {
  main().then(() => process.exit(0), error => { console.error(String(error.stack || error)); process.exit(1); });
}
module.exports = {fly};
