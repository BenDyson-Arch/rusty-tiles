// Browser acceptance for the issue #74 Rust-exported invented fixtures.
// Serve point and vector directories under /point-cloud and /annotations;
// each mount has explicit/, converted/ and fresh/ tilesets (see CONTRIBUTING).
// NODE_PATH exposes Playwright, CHROMIUM overrides installed Chromium.
// The fixed cameras use the source locations in tests/convert_implicit.rs.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
    headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    const runs = [];
    for (const load of ['initial', 'hard-refresh']) {
      if (load === 'initial') await page.goto(process.argv[2]);
      else {
        await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
        await page.reload({ waitUntil: 'load' });
      }
      await page.waitForFunction(() => window.annotations && window.pointCloud);
      const report = await page.evaluate(async () => {
        const C = Cesium, viewer = window.viewer;
        viewer.scene.globe.show = false;
        viewer.scene.skyAtmosphere.show = false;
        viewer.scene.skyBox.show = false;
        viewer.scene.primitives.remove(window.pointCloud);
        viewer.scene.primitives.remove(window.annotations);
        const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
        const outputs = [];
        for (const [kind, prefix] of [['point', '/point-cloud'], ['vector', '/annotations']]) {
          for (const variant of ['explicit', 'converted', 'fresh']) {
            const failures = [];
            const tiles = await C.Cesium3DTileset.fromUrl(`${prefix}/${variant}/tileset.json`, {
              maximumScreenSpaceError: 32, dynamicScreenSpaceError: false, foveatedScreenSpaceError: false,
            });
            tiles.tileFailed.addEventListener(error => failures.push(String(error.message || error)));
            viewer.scene.primitives.add(tiles);
            tiles.style = new C.Cesium3DTileStyle({ color: "color('cyan')", pointSize: 16 });
            function frame(multiplier) {
              const location = kind === 'point' ? [153.02, -27.47] : [12.05, 42.05];
              const height = kind === 'point' ? (multiplier === 300 ? 7000 : 2000) :
                (multiplier === 300 ? 500000 : 100000);
              viewer.camera.lookAtTransform(C.Matrix4.IDENTITY);
              viewer.camera.setView({ destination: C.Cartesian3.fromDegrees(...location, height),
                orientation: { heading: 0, pitch: -Math.PI / 2, roll: 0 } });
            }
            async function settle() {
              await wait(250);
              for (let i = 0; i < 150 && !tiles.tilesLoaded && !failures.length; i++) await wait(50);
              await wait(500);
            }
            async function inspect(picking) {
              const selected = [];
              const worlds = [];
              const picks = [];
              for (const tile of (tiles._selectedTiles || []).slice()) {
                for (const content of tile.content.innerContents || [tile.content]) {
                  if (!content.url?.endsWith('.glb')) continue;
                  const bytes = await (await fetch(content.url)).arrayBuffer();
                  const length = new DataView(bytes).getUint32(12, true);
                  const gltf = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 20, length)));
                  const node = gltf.nodes[gltf.scenes[gltf.scene || 0].nodes[0]];
                  const translation = node.translation || [0, 0, 0];
                  let count = 0;
                  for (const mesh of gltf.meshes) {
                    for (const primitive of mesh.primitives) {
                      const a = gltf.accessors[primitive.attributes.POSITION];
                      const v = gltf.bufferViews[a.bufferView];
                      const data = new DataView(bytes, 28 + length + (v.byteOffset || 0) + (a.byteOffset || 0));
                      if (a.componentType !== 5126) throw Error('Probe requires float32 fixture positions');
                      for (let i = 0; i < a.count; i++) {
                        const off = i * (v.byteStride || 12);
                        const xyz = [0, 1, 2].map(j => data.getFloat32(off + j * 4, true) + translation[j]);
                        const world = C.Matrix4.multiplyByPoint(tile.computedTransform,
                          new C.Cartesian3(xyz[0], -xyz[2], xyz[1]), new C.Cartesian3());
                        worlds.push([world.x, world.y, world.z]);
                        if (picking) {
                          const screen = C.SceneTransforms.worldToWindowCoordinates(viewer.scene, world);
                          const feature = screen && viewer.scene.pick(screen, 1, 1);
                          if (feature?.getPropertyIds) picks.push({
                            color: feature.color?.toCssColorString(),
                            properties: Object.fromEntries(feature.getPropertyIds().map(key =>
                              [key, String(feature.getProperty(key))])),
                          });
                        }
                      }
                      count += a.count;
                    }
                  }
                  selected.push({ uri: content.url.split('/').pop(), count,
                    nativeVector: Array.isArray(content._collections),
                    tileExtras: tile._header.extras, tileMetadataClass: tile.metadata?.class?.id,
                    contentExtras: tile._header.content?.extras });
                }
              }
              worlds.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
              const root = Array.from(tiles.root.computedTransform);
              let sourcePositionError = null;
              if (kind === 'point') {
                const expected = Array.from({ length: 16 }, (_, i) => {
                  const base = i < 8 ? -100 : 100;
                  const x = base + (i % 8) * .01 - .035;
                  const y = base + (i % 8) * .02 - .07;
                  const z = base + (i % 8) * .03 - .105;
                  const world = C.Matrix4.multiplyByPoint(tiles.root.computedTransform,
                    new C.Cartesian3(x, y, z), new C.Cartesian3());
                  return [world.x, world.y, world.z];
                }).sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
                if (picking && expected.length === worlds.length) sourcePositionError = Math.max(...worlds.map((p, i) =>
                  Math.hypot(...p.map((n, j) => n - expected[i][j]))));
              } else if (picking && worlds.length === 8) {
                const expected = Array.from({ length: 8 }, (_, i) => {
                  const base = i < 4 ? 12 : 12.1;
                  const p = C.Cartesian3.fromDegrees(base + i * .00001, base + 30 + i * .00001, 100);
                  return [p.x, p.y, p.z];
                }).sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
                sourcePositionError = Math.max(...worlds.map((p, i) =>
                  Math.hypot(...p.map((n, j) => n - expected[i][j]))));
              }
              return { selected, totalPoints: selected.reduce((sum, tile) => sum + tile.count, 0),
                rootTransform: root, worlds, sourcePositionError,
                picks: [...new Map(picks.map(pick => [JSON.stringify(pick.properties), pick])).values()],
                tilesLoaded: tiles.tilesLoaded, failures: [...new Set(failures)], viewerFailures: window.failures.slice(), useDefaultRenderLoop: viewer.useDefaultRenderLoop, camera: [C.Math.toDegrees(viewer.camera.positionCartographic.longitude), C.Math.toDegrees(viewer.camera.positionCartographic.latitude), viewer.camera.positionCartographic.height] };
            }
            try {
              frame(300); await settle(); const coarse = await inspect(false);
              tiles.maximumScreenSpaceError = 1e-7;
              frame(3); await settle(); const fine = await inspect(true);
              tiles.style = new C.Cesium3DTileStyle({ show: false });
              await wait(300);
              const hidden = await inspect(true);
              outputs.push({ kind, variant, coarse, fine, hiddenPicks: hidden.picks });
            } finally {
              viewer.scene.primitives.remove(tiles);
            }
          }
        }
        return { version: C.VERSION, outputs };
      });
      runs.push({ load, ...report });
    }
    console.log(JSON.stringify({ browserErrors: errors, runs }, null, 2));
    assert.deepEqual(errors, []);
    for (const run of runs) {
      assert.equal(run.version, '1.146.0');
      for (const result of run.outputs) {
        assert.deepEqual(result.coarse.failures, []);
        assert.deepEqual(result.fine.failures, []);
        assert.deepEqual(result.fine.viewerFailures, []);
        assert.equal(result.fine.tilesLoaded, true);
        assert.equal(result.fine.totalPoints, result.kind === 'point' ? 16 : 8);
        if (result.kind === 'point') assert.ok(result.fine.totalPoints > result.coarse.totalPoints, 'Cloud LOD did not refine');

        assert.ok(result.fine.picks.length >= 2, 'Both source clusters must render/pick');
        assert.deepEqual(result.hiddenPicks, []);
        assert.ok(result.fine.sourcePositionError !== null && result.fine.sourcePositionError < .002, 'World position does not match source/root frame');
        for (const pick of result.fine.picks) {
          assert.equal(pick.color, 'rgb(0,255,255)');
          const p = pick.properties;
          if (result.kind === 'point') {
            const id = Number(p.source_index);
            assert.ok(Number.isInteger(id) && id >= 0 && id < 16);
            assert.equal(Number(p.intensity), 100 + id);
            assert.equal(Number(p.classification), 42);
          } else {
            const id = Number(p.value);
            assert.equal(p.name, `point-${id}`);
            assert.equal(p._source_id, String(id));
            assert.ok(p._source_layer);
          }
        }
      }
      for (const kind of ['point', 'vector']) {
        const outputs = run.outputs.filter(out => out.kind === kind);
        assert.deepEqual(outputs[0].fine.rootTransform, outputs[1].fine.rootTransform);
        for (const compared of outputs.slice(1)) {
          const maxError = Math.max(...outputs[0].fine.worlds.map((p, i) =>
            Math.hypot(...p.map((n, j) => n - compared.fine.worlds[i][j]))));
          assert.ok(maxError < 1e-5, `${kind}/${compared.variant} placement changed: ${maxError}`);
        }
      }
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
