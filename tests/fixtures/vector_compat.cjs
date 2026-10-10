// Optional browser probe of the repository preview's CesiumJS IIFE runtime.
// NODE_PATH must expose Playwright. This script downloads nothing.
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
    headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
    const browserErrors = [];
    page.on('pageerror', error => browserErrors.push(String(error)));
    const network = await page.context().newCDPSession(page);
    await network.send('Network.enable');
    for (const load of ['initial', 'hard-refresh']) {
      if (load === 'hard-refresh') {
        await network.send('Network.setCacheDisabled', {cacheDisabled: true});
        await page.reload({waitUntil: 'load'});
      } else {
        await page.goto(process.argv[2] || 'http://127.0.0.1:9250');
      }
    await page.waitForFunction(() => window.annotations || window.failures?.length,
      { timeout: 30000 });
    const results = await page.evaluate(async () => {
      const C = Cesium, viewer = window.viewer;
      viewer.scene.globe.show = false;
      viewer.scene.skyAtmosphere.show = false;
      viewer.scene.skyBox.show = false;
      viewer.scene.backgroundColor = C.Color.BLACK;
      if (window.annotations) viewer.scene.primitives.remove(window.annotations);
      const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
      const cases = await (await fetch('/annotations/cases.json')).json();
      const output = [];

      function screen(position) {
        return C.SceneTransforms.worldToWindowCoordinates(viewer.scene,
          C.Cartesian3.fromDegrees(...position));
      }
      function pickScreen(position) {
        const feature = position && viewer.scene.pick(position, 1, 1);
        const properties = feature?.getPropertyIds ? Object.fromEntries(
          feature.getPropertyIds().map(key => [key, String(feature.getProperty(key))])) : {};
        return { rendered: !!feature, properties };
      }
      function pick(position, offsetY = 0) {
        const windowPosition = screen(position);
        if (windowPosition) windowPosition.y += offsetY;
        return pickScreen(windowPosition);
      }
      function outsideBoundary(entry) {
        if (!entry.boundary) return null;
        const windowPosition = screen(entry.boundary), center = screen(entry.hole);
        const dx = windowPosition.x - center.x, dy = windowPosition.y - center.y;
        const length = Math.hypot(dx, dy);
        windowPosition.x += 5 * dx / length;
        windowPosition.y += 5 * dy / length;
        return pickScreen(windowPosition);
      }
      async function inspect(tiles, entry) {
        let vertices = 0, gltfPrimitives = 0;
        const decoders = [];
        const hierarchyLevels = new Set();
        function walk(tile, offset = 0) {
          const uri = tile.implicitTileset?.baseResource?.url;
          const name = uri?.split('/').pop();
          if (name?.startsWith('implicit-tileset-')) {
            const parts = name.slice(17).replace('.json', '').split('-').map(Number);
            offset = parts[0] + parts[3];
          }
          if (tile.implicitCoordinates) hierarchyLevels.add(offset + tile.implicitCoordinates.level);
          for (const child of tile.children) walk(child, offset);
        }
        walk(tiles.root);
        // Private traversal/collection fields are diagnostics confined to this probe.
        for (const tile of tiles._selectedTiles || []) {
          for (const content of tile.content.innerContents || [tile.content]) {
            decoders.push({
              nativeVector: Array.isArray(content._collections),
              collections: (content._collections || []).map(collection => ({
                kind: collection instanceof C.BufferPolygonCollection ? 'polygon' :
                  collection instanceof C.BufferPolylineCollection ? 'line' : 'point',
                primitives: collection.primitiveCount,
              })),
            });
            if (!content.url) continue;
            let bytes = new Uint8Array(await (await fetch(content.url)).arrayBuffer());
            if (new TextDecoder().decode(bytes.slice(0, 4)) === 'b3dm') {
              const header = new DataView(bytes.buffer);
              let offset = 28;
              for (let i = 12; i <= 24; i += 4) offset += header.getUint32(i, true);
              bytes = bytes.slice(offset);
            }
            const jsonLength = new DataView(bytes.buffer, bytes.byteOffset,
              bytes.byteLength).getUint32(12, true);
            const document = JSON.parse(new TextDecoder().decode(bytes.slice(20, 20 + jsonLength)));
            gltfPrimitives += document.meshes[0].primitives.length;
            vertices += document.meshes[0].primitives.reduce((sum, primitive) =>
              sum + document.accessors[primitive.attributes.POSITION].count, 0);
          }
        }
        return {
          selectedTiles: tiles._selectedTiles?.length || 0, vertices, gltfPrimitives, decoders,
          hierarchyLevels: [...hierarchyLevels].sort((a, b) => a - b),
          pick: pick(entry.sample),
          samples: (entry.samples || []).map(sample => ({ expectedId: sample.id, ...pick(sample.position) })),
          gap: entry.gap ? pick(entry.gap) : null,
          wideLine: ['line', 'outline'].includes(entry.name) ?
            { negative: pick(entry.sample, -5), positive: pick(entry.sample, 5) } : null,
          boundary: outsideBoundary(entry),
          hole: entry.hole ? pick(entry.hole) : null,
          failures: window.failures.slice(),
        };
      }
      for (const entry of cases) {
        window.failures = [];
        let tiles;
        try {
          tiles = await C.Cesium3DTileset.fromUrl(`/annotations/${entry.name}/tileset.json`, {
            maximumScreenSpaceError: 32, backFaceCulling: false,
            dynamicScreenSpaceError: false, foveatedScreenSpaceError: false,
          });
          viewer.scene.primitives.add(tiles);
          tiles.tileFailed.addEventListener(error => window.failures.push(String(error.message || error)));
          tiles.style = new C.Cesium3DTileStyle({
            color: "color('cyan', 0.8)", pointSize: 12, lineWidth: 16,
          });
          function frame(distance) {
            viewer.camera.viewBoundingSphere(tiles.boundingSphere,
              new C.HeadingPitchRange(0, -Math.PI / 2, distance));
            viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
          }
          async function settle() {
            await wait(100); // Let traversal issue requests before checking tilesLoaded.
            for (let i = 0; i < 150 && !tiles.tilesLoaded && !window.failures.length; i++) {
              await wait(50);
            }
            await wait(400);
          }
          frame(entry.name === 'point-aggregates' ?
            Math.max(tiles.boundingSphere.radius * 600, 1000) :
            Math.max(tiles.boundingSphere.radius * 30, 60));
          await settle();
          const coarse = await inspect(tiles, entry);
          tiles.maximumScreenSpaceError = 1e-7;
          frame(Math.max(tiles.boundingSphere.radius * 3, 5));
          await settle();
          output.push({ case: entry.name, coarse, fine: await inspect(tiles, entry) });
        } catch (error) {
          output.push({ case: entry.name, error: String(error), failures: window.failures.slice() });
        } finally {
          if (tiles) viewer.scene.primitives.remove(tiles);
        }
      }
      return { version: C.VERSION, cases: output };
    });
    results.load = load;
    results.browserErrors = browserErrors;
    process.stdout.write(JSON.stringify(results, null, 2) + '\n');
    if (process.argv.includes('--require-native')) {
      const picked = result => result?.rendered && result.properties?._source_id &&
        result.properties?._source_layer && result.properties?.name;
      const expectedCases = process.argv.includes('--require-aggregates') ? 6 : 5;
      const passed = !browserErrors.length && results.cases.length === expectedCases &&
        results.cases.every(entry => {
          const fine = entry.fine;
          return !entry.error && fine && !entry.coarse.failures.length &&
            fine.hierarchyLevels.length > 0 && fine.hierarchyLevels.every((level, index) => level === index) &&
            !fine.failures.length && picked(fine.pick) &&
            fine.samples.every(sample => picked(sample) && sample.properties._source_id === sample.expectedId) &&
            (!fine.gap || !fine.gap.rendered) &&
            (!fine.samples.length || entry.case === 'fragmented' ||
              (fine.gltfPrimitives === 1 && fine.decoders.length === 1 &&
                fine.decoders[0].collections.length === 1 &&
                fine.decoders[0].collections[0].primitives === 2)) &&
            fine.decoders.some(decoder => decoder.nativeVector) &&
            (entry.case !== 'point-aggregates' ||
              (entry.coarse.pick.rendered && entry.coarse.pick.properties.aggregation === 'voxel' &&
                Number(entry.coarse.pick.properties.pointCount) > 1 &&
                !entry.coarse.pick.properties._source_id &&
                fine.vertices > entry.coarse.vertices &&
                !fine.pick.properties.aggregation)) &&
            (!fine.hole || !fine.hole.rendered) &&
            (!fine.wideLine || picked(fine.wideLine.negative) || picked(fine.wideLine.positive)) &&
            (!fine.boundary || picked(fine.boundary)) &&
            (!['line', 'polygon'].includes(entry.case) || fine.vertices > entry.coarse.vertices);
        });
      if (!passed) {
        console.error('Native vector load/render/LOD/picking check failed; see JSON report.');
        process.exitCode = 1;
      }
    }
    }
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
