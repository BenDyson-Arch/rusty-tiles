// Issue #75: identical fragmented fills with/without b3dm in Cesium 1.146.
// NODE_PATH exposes Playwright; CHROMIUM may override installed Chromium.
// --expect-current-failure locks down the known decoder limitation. Without it,
// this is an acceptance gate for a future Cesium release that fixes the issue.
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
    const browserErrors = [];
    page.on('pageerror', error => browserErrors.push(String(error)));
    const network = await page.context().newCDPSession(page);
    await network.send('Network.enable');
    const runs = [];
    for (const load of ['initial', 'hard-refresh']) {
      if (load === 'initial') await page.goto(process.argv[2]);
      else {
        await network.send('Network.setCacheDisabled', { cacheDisabled: true });
        await page.reload({ waitUntil: 'load' });
      }
      await page.waitForFunction(() => window.annotations || window.failures?.length);
      const result = await page.evaluate(async () => {
        const C = Cesium, viewer = window.viewer;
        viewer.scene.globe.show = false;
        viewer.scene.skyAtmosphere.show = false;
        viewer.scene.skyBox.show = false;
        viewer.scene.backgroundColor = C.Color.BLACK;
        if (window.annotations) viewer.scene.primitives.remove(window.annotations);
        const fixture = await (await fetch('/annotations/fixture.json')).json();
        const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
        function pick(position) {
          const pixel = C.SceneTransforms.worldToWindowCoordinates(viewer.scene,
            C.Cartesian3.fromDegrees(...position));
          const feature = pixel && viewer.scene.pick(pixel, 1, 1);
          return { rendered: !!feature, color: feature?.color?.toCssColorString(),
            properties: feature?.getPropertyIds ? Object.fromEntries(
              feature.getPropertyIds().map(key => [key, String(feature.getProperty(key))])) : {} };
        }
        function boundary(entry) {
          if (!entry.boundary) return null;
          const point = C.SceneTransforms.worldToWindowCoordinates(viewer.scene,
            C.Cartesian3.fromDegrees(...entry.boundary));
          const center = C.SceneTransforms.worldToWindowCoordinates(viewer.scene,
            C.Cartesian3.fromDegrees(...entry.hole));
          const dx = point.x - center.x, dy = point.y - center.y;
          const length = Math.hypot(dx, dy);
          point.x += 5 * dx / length;
          point.y += 5 * dy / length;
          const feature = viewer.scene.pick(point, 1, 1);
          return { rendered: !!feature, sourceId: feature?.getProperty?.('_source_id') };
        }
        const cases = [];
        for (const format of ['wrapped', 'plain']) {
          for (const entry of fixture.cases) {
            let tiles;
            const failures = [];
            try {
              tiles = await C.Cesium3DTileset.fromUrl(
                `/annotations/${format}/${entry.name}/tileset.json`, {
                  maximumScreenSpaceError: 32, backFaceCulling: false,
                  dynamicScreenSpaceError: false, foveatedScreenSpaceError: false,
                });
              viewer.scene.primitives.add(tiles);
              tiles.tileFailed.addEventListener(error => failures.push(String(error.message || error)));
              const name = entry.propertyName || entry.name;
              tiles.style = new C.Cesium3DTileStyle({
                color: { conditions: [[`\${name} === '${name}'`, "color('cyan')"], ['true', "color('orange')"]] },
                pointSize: 12, lineWidth: 16,
              });
              function frame(fine) {
                if (entry.country) {
                  viewer.camera.setView({ destination: C.Cartesian3.fromDegrees(
                    entry.sample[0], entry.sample[1], fine ? 2000000 : 20000000),
                    orientation: { heading: 0, pitch: -Math.PI / 2, roll: 0 } });
                } else {
                  viewer.camera.viewBoundingSphere(tiles.boundingSphere,
                    new C.HeadingPitchRange(0, -Math.PI / 2,
                      Math.max(tiles.boundingSphere.radius * (fine ? 3 : 30), fine ? 5 : 60)));
                  viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
                }
              }
              async function settle() {
                await wait(100);
                for (let i = 0; i < 150 && !tiles.tilesLoaded && !failures.length; i++) await wait(50);
                await wait(400);
              }
              function inspect() {
                const contents = (tiles._selectedTiles || []).flatMap(tile =>
                  tile.content.innerContents || [tile.content]);
                const decoders = new Map();
                for (const content of contents) {
                  const nativeVector = Array.isArray(content._collections);
                  const format = content.url?.split('.').pop();
                  const key = `${nativeVector}:${format}`;
                  if (!decoders.has(key)) decoders.set(key, {
                    nativeVector, format, contents: 0, triangles: 0, points: 0,
                  });
                  const decoder = decoders.get(key);
                  decoder.contents++;
                  decoder.triangles += content.trianglesLength || 0;
                  decoder.points += content.pointsLength || 0;
                }
                return { pick: pick(entry.sample),
                  samples: (entry.samples || []).map(sample => ({ expectedId: sample.id, ...pick(sample.position) })),
                  hole: entry.hole ? pick(entry.hole) : null,
                  boundary: boundary(entry), gap: entry.gap ? pick(entry.gap) : null,
                  selectedTiles: tiles._selectedTiles?.length || 0,
                  decoders: [...decoders.values()],
                  failures: [...new Set(failures)] };
              }
              frame(false);
              await settle();
              const coarse = inspect();
              tiles.maximumScreenSpaceError = 1e-7;
              frame(true);
              await settle();
              const fine = inspect();
              tiles.style = new C.Cesium3DTileStyle({ show: `\${name} !== '${name}'` });
              await wait(300);
              cases.push({ format, case: entry.name, propertyName: name, expectedId: entry.expectedId,
                coarse, fine, hidden: pick(entry.sample) });
            } catch (error) {
              cases.push({ format, case: entry.name, error: String(error), failures });
            } finally {
              if (tiles) viewer.scene.primitives.remove(tiles);
            }
          }
        }
        return { version: C.VERSION, compressed: fixture.compressed,
          unwrappedContents: fixture.unwrappedContents,
          countrySourceSha256: fixture.countrySourceSha256, cases };
      });
      runs.push({ load, ...result });
    }
    const knownFailure = process.argv.includes('--expect-current-failure');
    const picked = pick => pick?.rendered && pick.properties?._source_id &&
      pick.properties?._source_layer && pick.properties?.name;
    const styled = (pick, name) => picked(pick) && pick.color ===
      (pick.properties.name === name ? 'rgb(0,255,255)' : 'rgb(255,165,0)');
    let passed = !browserErrors.length;
    for (const run of runs) {
      passed &&= run.cases.length === 14;
      for (const entry of run.cases) {
        const fails = knownFailure && entry.format === 'plain' &&
          ['fragmented', 'sudan', 'antarctica'].includes(entry.case);
        if (fails) {
          // Antarctica can retain a coarser native polygon as a fallback while
          // some fragmented descendants fail. A pick alone misses lost detail.
          passed &&= !entry.error && (entry.case === 'antarctica' || !entry.fine.pick.rendered) &&
            entry.fine.failures.includes("Cannot read properties of undefined (reading 'loopIndices')");
        } else {
          const fine = entry.fine;
          passed &&= !entry.error && !!fine && !fine.failures.length && !entry.coarse.failures.length &&
            styled(fine.pick, entry.propertyName) && !entry.hidden.rendered &&
            (!entry.expectedId || fine.pick.properties._source_id === entry.expectedId) &&
            fine.samples.every(sample => styled(sample, entry.propertyName) && sample.properties._source_id === sample.expectedId) &&
            (!fine.hole || !fine.hole.rendered) && (!fine.gap || !fine.gap.rendered) &&
            (!fine.boundary || fine.boundary.rendered) &&
            (!['line', 'polygon'].includes(entry.case) ||
              fine.decoders.reduce((sum, decoder) => sum + decoder.triangles, 0) >
              entry.coarse.decoders.reduce((sum, decoder) => sum + decoder.triangles, 0));
        }
      }
    }
    process.stdout.write(JSON.stringify({ expectedPlainFailure: knownFailure,
      passed, browserErrors, runs }, null, 2) + '\n');
    if (!passed) process.exitCode = 1;
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
