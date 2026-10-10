// Optional upstream validation with an explicit workaround for the 0.5.0
// traverser's single-template bug. No schemas or validators are replaced.
// Usage: node implicit_validator.cjs VALIDATOR_PACKAGE TILESET... [--fix-multiple-content-traversal]
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createRequire} = require('node:module');
const [packageDirectory, ...arguments_] = process.argv.slice(2);
assert.ok(packageDirectory && arguments_.length, 'Pass an installed validator package and tileset paths');
const packagePath = path.resolve(packageDirectory);
const localRequire = createRequire(path.join(packagePath, 'package.json'));
const version = JSON.parse(fs.readFileSync(path.join(packagePath, 'package.json'))).version;
const {Validators} = require(path.join(packagePath, 'build/index.js'));
const tools = localRequire('3d-tiles-tools');
const fix = arguments_.includes('--fix-multiple-content-traversal');
if (fix) {
  assert.equal(version, '0.6.1', 'Review the workaround before using another validator version');
  const toolsPath = path.dirname(localRequire.resolve('3d-tiles-tools'));
  const toolsVersion = JSON.parse(fs.readFileSync(path.join(toolsPath, '../../package.json'))).version;
  assert.equal(toolsVersion, '0.5.0');
  tools.ImplicitTraversedTile.prototype.getRawContents = function () {
    const root = this._root.asRawTile();
    const templates = root.contents || (root.content ? [root.content] : []);
    const index = this._localCoordinate.toIndex();
    return this._subtreeModel.subtreeInfo.contentAvailabilityInfos.flatMap((availability, slot) => {
      if (!availability.isAvailable(index)) return [];
      assert.ok(templates[slot]?.uri, `Missing template for content slot ${slot}`);
      return [{...templates[slot], uri: tools.ImplicitTilings.substituteTemplateUri(
        this._implicitTiling.subdivisionScheme, templates[slot].uri, this._globalCoordinate)}];
    });
  };
}
(async () => {
  for (const file of arguments_.filter(a => !a.startsWith('--'))) {
    const result = await Validators.validateTilesetFile(path.resolve(file));
    console.log(JSON.stringify({validator: version, multipleContentTraversalFix: fix,
      file, errors: result.numErrors, warnings: result.numWarnings, infos: result.numInfos}));
    if (result.numErrors) {
      console.error(JSON.stringify(result, null, 2));
      process.exitCode = 1;
    }
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
