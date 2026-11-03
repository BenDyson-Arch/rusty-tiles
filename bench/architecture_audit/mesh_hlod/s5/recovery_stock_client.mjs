// Headless loading/refinement control using unmodified stock Cesium traversal.
// Usage: node recovery_stock_client.mjs /path/to/node_modules/@cesium/engine
// This tests traversal with controlled SSE/visibility/content states, not WebGL,
// network scheduling, camera-bound calculations or rendered image quality.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const engine = resolve(process.argv[2]);
const files = ['Source/Scene/Cesium3DTilesetBaseTraversal.js',
  'Source/Scene/Cesium3DTilesetTraversal.js', 'Source/Scene/Cesium3DTileRefine.js'];
const { default: traversal } = await import(pathToFileURL(resolve(engine, files[0])));
const { default: refine } = await import(pathToFileURL(resolve(engine, files[2])));
const hashes = Object.fromEntries(await Promise.all(files.map(async file =>
  [file, createHash('sha256').update(await readFile(resolve(engine, file))).digest('hex')])));
const results = [];

function fixture({ linked = false, empty = false } = {}) {
  const set = {
    memoryAdjustedScreenSpaceError: 16, _requestedTiles: [], _selectedTiles: [],
    _selectedTilesToStyle: [], _emptyTiles: [], _statistics: { visited: 0 },
    _cache: { touch() {} }, _minimumPriority: {}, _maximumPriority: {},
    _cullRequestsWhileMoving: false, hasExtension() { return false; },
    foveatedTimeDelay: 0,
  };
  function tile(name, sse, ownerDepth, renderable = true) {
    return {
      name, tileset: set, children: [], refine: refine.REPLACE, ownershipDepth: ownerDepth,
      _depth: 1, _screenSpaceError: sse, _visible: true, _inRequestVolume: true,
      _distanceToCamera: 100, _centerZDepth: 0, _foveatedFactor: 0,
      _priorityReverseScreenSpaceError: 0, _selectedFrame: -1,
      hasRenderableContent: renderable, hasEmptyContent: !renderable,
      hasTilesetContent: false, hasImplicitContent: false,
      contentAvailable: true, contentExpired: false, content: {},
      get hasUnloadedRenderableContent() { return this.hasRenderableContent && !this.contentAvailable; },
      get isVisible() { return this._visible; },
      getScreenSpaceError() { return 1000; }, // Root document's top-level SSE.
      updateVisibility() {}, updateExpiration() {}, updatePriority() {},
      contentVisibility() { return 1; },
    };
  }
  const root = tile('root', 32, 0);
  root._depth = 0;
  const frontier = [1, 1, 1, 2, 2, 2, 2].map((d, i) => tile(`frontier-${i}`, 8, d));
  root.children = frontier;
  for (const child of frontier) child.parent = root;
  if (linked) {
    const link = tile('document-link', 8, 2, false);
    link.hasEmptyContent = false;
    link.hasTilesetContent = true;
    link.contentAvailable = false;
    link.children = [frontier[6]];
    link.parent = root;
    frontier[6].parent = link;
    root.children = [...frontier.slice(0, 6), link];
  }
  if (empty) {
    const gap = tile('omitted-routing-cell', 8, 1, false);
    gap.contentAvailable = false;
    gap.children = frontier;
    gap.parent = root;
    for (const child of frontier) child.parent = gap;
    root.children = [gap];
  }
  set.root = root;
  let frame = 0;
  return { set, root, frontier, run(name, want) {
    traversal.selectTiles(set, { frameNumber: ++frame, camera: { timeSinceMoved: 10 } });
    const selected = set._selectedTiles.map(t => t.name).sort();
    assert.deepEqual(selected, want.slice().sort(), name);
    results.push({ name, selected, requested: set._requestedTiles.map(t => t.name).sort() });
  } };
}

for (const linked of [false, true]) {
  const f = fixture({ linked });
  const tag = linked ? 'linked' : 'inline';
  f.root._screenSpaceError = 8;
  for (const child of f.frontier) child.contentAvailable = false;
  f.run(`${tag}: below SSE`, ['root']);
  f.root._screenSpaceError = 32;
  f.run(`${tag}: all frontier pending`, ['root']);
  f.frontier[0].contentAvailable = true;
  f.frontier[3].contentAvailable = true;
  f.run(`${tag}: partial frontier ready`, ['root']);
  for (const child of f.frontier) child.contentAvailable = true;
  f.run(`${tag}: frontier ready`, f.frontier.map(t => t.name));
  f.frontier[1]._visible = false;
  f.frontier[1].contentAvailable = false;
  f.run(`${tag}: nonvisible frontier pending`, ['root']);
  assert(f.set._requestedTiles.includes(f.frontier[1]));
}
const gap = fixture({ empty: true });
gap.run('control: empty routing cell stops below SSE', []);
console.log(JSON.stringify({ engine: JSON.parse(await readFile(resolve(engine, 'package.json'))).version,
  sourceSha256: hashes, results,
  limits: 'Controlled traversal states; no browser, GPU, network or rendered-image assertion.' }, null, 2));
