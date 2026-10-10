// Read invented library-writer fixtures with Cesium's own subtree parser.
// This checks availability and subtree links; it does not test rendering.
// Usage: node tests/fixtures/implicit_subtree.cjs FIXTURES CESIUM_PACKAGE_DIR
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const [fixtures, runtime] = process.argv.slice(2);
if (!fixtures || !runtime) {
  throw new Error('Pass the fixture directory and a pinned Cesium package directory');
}
const Cesium = require(path.resolve(runtime));

async function main() {
  let checked = 0;
  for (const [scheme, contentCount] of [['QUADTREE', 1], ['QUADTREE', 2], ['OCTREE', 1], ['OCTREE', 2]]) {
    const octree = scheme === 'OCTREE';
    const branches = octree ? 8 : 4;
    const directory = path.resolve(fixtures, `${contentCount === 1 ? 'single-' : ''}${scheme.toLowerCase()}`);
    const doc = JSON.parse(fs.readFileSync(path.join(directory, 'tileset.json')));
    const base = new Cesium.Resource({url: 'http://127.0.0.1/tileset.json'});
    const tileset = new Cesium.ImplicitTileset(base, doc.root);
    const cases = [
      {key: octree ? '0-0-0-0' : '0-0-0', coordinates: {level: 0, x: 0, y: 0, z: 0},
        tiles: [0, octree ? 6 : 2], contents: [[0], [octree ? 6 : 2]], children: [octree ? 41 : 5]},
      {key: octree ? '2-3-0-2' : '2-3-0', coordinates: {level: 2, x: 3, y: 0, z: octree ? 2 : 0},
        tiles: [0, 4], contents: [[0, 4], [0]], children: []},
    ];
    for (const expected of cases) {
      const coordinates = new Cesium.ImplicitTileCoordinates({
        subdivisionScheme: scheme, subtreeLevels: 2, ...expected.coordinates,
      });
      const bytes = fs.readFileSync(path.join(directory, 'subtrees', `${expected.key}.subtree`));
      const subtree = await Cesium.ImplicitSubtree.fromSubtreeJson(
        base, undefined, new Uint8Array(bytes), tileset, coordinates,
      );
      const indices = (length, predicate) => Array.from({length}, (_, i) => i).filter(predicate);
      assert.deepEqual(indices(branches + 1, i => subtree.tileIsAvailableAtIndex(i)), expected.tiles);
      for (let slot = 0; slot < contentCount; slot++) {
        assert.deepEqual(indices(branches + 1, i => subtree.contentIsAvailableAtIndex(i, slot)), expected.contents[slot]);
      }
      assert.deepEqual(indices(branches * branches, i => subtree.childSubtreeIsAvailableAtIndex(i)), expected.children);
      assert.equal(subtree.tileIsAvailableAtCoordinates(coordinates), true);
      checked++;
      subtree.destroy();
    }
  }
  process.stdout.write(JSON.stringify({cesium: Cesium.VERSION, subtrees: checked, result: 'passed'}) + '\n');
}

main().catch(error => { console.error(error); process.exitCode = 1; });
