// P-IMPL layout probe: load one fixture variant in pinned Cesium, settle an overview and one
// close top-down view per leaf, and report selection, requests, center pick and pixel coverage.
// Usage: NODE_PATH=... CHROMIUM=... node probe.cjs PAGE_URL SPEC_JSON SCREENSHOT_PREFIX
const {chromium} = require('playwright');
const fs = require('node:fs');

async function run(spec) {
  const C = Cesium, failures = [];
  const viewer = new C.Viewer('view', {globe: false, baseLayer: false, skyBox: false, skyAtmosphere: false,
    geocoder: false, baseLayerPicker: false, animation: false, timeline: false, sceneModePicker: false,
    navigationHelpButton: false, homeButton: false, fullscreenButton: false, selectionIndicator: false,
    infoBox: false, contextOptions: {webgl: {preserveDrawingBuffer: true}}});
  const scene = viewer.scene;
  scene.postProcessStages.fxaa.enabled = false;
  scene.backgroundColor = C.Color.BLACK;
  performance.setResourceTimingBufferSize(100000);
  scene.renderError.addEventListener((u, e) => failures.push('renderError: ' + e));
  const settle = ready => new Promise(resolve => {
    let stable = 0, remove = () => {};
    const timer = setTimeout(() => { remove(); resolve(false); }, spec.stepMs);
    remove = scene.postRender.addEventListener(() => {
      stable = ready() ? stable + 1 : 0;
      if (stable >= (spec.stableFrames || 8) || failures.length) { remove(); clearTimeout(timer); resolve(stable >= (spec.stableFrames || 8)); }
      else scene.requestRender();
    });
    scene.requestRender();
  });
  const base = new URL(spec.url, location.href).href.replace(/tileset\.json$/, '');
  const rel = url => url ? decodeURI(String(url)).replace(base, '') : null;
  const fixtureRequests = () => performance.getEntriesByType('resource').map(e => e.name).filter(n => n.startsWith(base)).map(rel);
  const start = performance.now();
  const options = {maximumScreenSpaceError: spec.maximumScreenSpaceError || 16};
  // Deterministic mode: no camera-dependent request deferral, so a settled frame is a steady state.
  if (spec.deterministic) Object.assign(options, {foveatedScreenSpaceError: false, dynamicScreenSpaceError: false,
    progressiveResolutionHeightFraction: 0, cullRequestsWhileMoving: false, preloadWhenHidden: false});
  if (spec.cullWithChildrenBounds === false) options.cullWithChildrenBounds = false;
  const tiles = await C.Cesium3DTileset.fromUrl(spec.url, options);
  tiles.tileFailed.addEventListener(e => failures.push('tileFailed: ' + (e.message || e)));
  scene.primitives.add(tiles);
  const sphere = tiles.boundingSphere;
  viewer.camera.viewBoundingSphere(sphere, new C.HeadingPitchRange(-.8, -.65, Math.max(sphere.radius * 3, 5)));
  viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
  const overviewLoaded = await settle(() => tiles.tilesLoaded);
  const overview = {loaded: overviewLoaded, seconds: (performance.now() - start) / 1000, requests: fixtureRequests(),
    selected: tiles._selectedTiles.map(t => rel(t.content && t.content.url))};
  const pixels = () => {
    const canvas = document.createElement('canvas');
    canvas.width = scene.canvas.width; canvas.height = scene.canvas.height;
    const context = canvas.getContext('2d');
    context.drawImage(scene.canvas, 0, 0);
    return context.getImageData(0, 0, canvas.width, canvas.height);
  };
  const views = [];
  for (const view of spec.views) {
    const before = fixtureRequests().length;
    const t0 = performance.now();
    const target = C.Matrix4.multiplyByPoint(tiles.root.computedTransform, new C.Cartesian3(view.target[0], view.target[1], view.target[2] || 0), new C.Cartesian3());
    viewer.camera.lookAt(target, new C.HeadingPitchRange(0, -Math.PI / 2 + 1e-4, view.height || spec.height));
    viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
    const loaded = await settle(() => tiles.tilesLoaded);
    const seconds = (performance.now() - t0) / 1000;
    const selected = tiles._selectedTiles.map(t => rel(t.content && t.content.url));
    const w = scene.canvas.clientWidth, h = scene.canvas.clientHeight;
    const picked = scene.pick(new C.Cartesian2(w / 2, h / 2));
    const pickedUrl = picked && picked.content ? rel(picked.content.url) : (picked ? 'non-content:' + Object.keys(picked).join(',') : null);
    scene.render();
    const image = pixels(), scale = image.width / w;
    let samples = 0, content = 0, leafColor = 0;
    for (let i = 0; i < 15; i++) for (let j = 0; j < 15; j++) {
      const px = Math.floor((.2 + .6 * i / 14) * w * scale), py = Math.floor((.2 + .6 * j / 14) * h * scale);
      const k = (py * image.width + px) * 4, r = image.data[k], g = image.data[k + 1], b = image.data[k + 2];
      samples++;
      if (Math.max(r, g, b) > 8) content++;
      if (r > 200 && g > 200 && b < 60) leafColor++;
    }
    const expected = view.leafUri;
    views.push({name: view.name, control: !!view.control, routingAncestor: view.routingAncestor,
      distanceToPlaceholderCell: view.distanceToPlaceholderCell, loaded, seconds, selected,
      leafSelected: selected.includes(expected), pickedUrl, pickedLeaf: pickedUrl === expected,
      coverage: content / samples, leafColorFraction: leafColor / samples,
      newRequests: fixtureRequests().slice(before)});
  }
  const all = fixtureRequests();
  const count = ext => all.filter(n => n.endsWith(ext)).length;
  return {version: C.VERSION, failures, overview, views,
    requests: {total: all.length, unique: new Set(all).size, json: count('.json'), subtree: count('.subtree'), glb: count('.glb')},
    statistics: {numberOfTilesTotal: tiles.statistics.numberOfTilesTotal}};
}

(async () => {
  const [url, specText, shots] = process.argv.slice(2), spec = JSON.parse(specText);
  const browser = await chromium.launch({executablePath: process.env.CHROMIUM, headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']});
  const errors = [], failedRequests = [];
  try {
    const page = await browser.newPage({viewport: {width: 800, height: 600}});
    page.setDefaultTimeout(spec.stepMs);
    page.on('pageerror', e => errors.push(String(e)));
    page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
    page.on('response', r => { if (r.status() >= 400) failedRequests.push({url: r.url(), status: r.status()}); });
    await page.goto(url);
    await page.waitForFunction(() => window.Cesium);
    await page.addScriptTag({content: run.toString()});
    const result = await page.evaluate(async spec => {
      try { return await run(spec); } catch (e) { return {failures: ['threw: ' + (e.stack || e)]}; }
    }, spec);
    await page.screenshot({path: shots + '-last.png'});
    console.log(JSON.stringify({...result, browserVersion: browser.version(), errors, failedRequests}));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
