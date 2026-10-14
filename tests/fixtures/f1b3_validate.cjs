// Official Khronos validation supplements the independent corner/resource oracle.
// Pass validator node_modules, a confined root, and source/leaf paths within root.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const [modules, rootArgument, ...requestedInputs] = process.argv.slice(2);
assert(modules && rootArgument, 'validator modules and confined root required');
const root = fs.realpathSync(rootArgument);
const inputs = requestedInputs.length ? requestedInputs : JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'))).fixtures.map(f => f.source);
assert(inputs.length, 'at least one source/leaf required');
const packageDirectory = path.resolve(modules, 'gltf-validator');
const validator = require(packageDirectory);
const hash = data => crypto.createHash('sha256').update(data).digest('hex');
(async () => {
  const results = [];
  for (const input of inputs) {
    const filename = fs.realpathSync(path.resolve(root, input));
    assert(filename.startsWith(root + path.sep), 'input escapes validation root');
    const resources = [];
    const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(filename)), {
      uri: path.relative(root, filename).split(path.sep).join('/'), maxIssues: 0, writeTimestamp: false,
      externalResourceFunction: async uri => {
        assert(!/^[a-z][a-z0-9+.-]*:/i.test(uri) && !uri.startsWith('/') && !/[?#\\]/.test(uri), 'unexpected resource URI');
        const components = uri.split('/').map(part => decodeURIComponent(part));
        assert(components.every(part => !/[\\/\x00]/.test(part)), 'encoded resource separator');
        const resource = fs.realpathSync(path.resolve(path.dirname(filename), ...components));
        assert(resource.startsWith(root + path.sep), 'resource escapes validation root');
        const data = fs.readFileSync(resource);
        resources.push({uri, path: path.relative(root, resource), bytes: data.length, sha256: hash(data)});
        return new Uint8Array(data);
      },
    });
    results.push({input, source_sha256: hash(fs.readFileSync(filename)), resources, report});
  }
  const manifest = fs.readFileSync(path.join(packageDirectory, 'package.json'));
  const receipt = {validator_version: validator.version(), authored_driver_sha256: hash(fs.readFileSync(__filename)), package_sha256: hash(manifest),
    entry_sha256: hash(fs.readFileSync(path.join(packageDirectory, 'index.js'))), results};
  console.log(JSON.stringify(receipt, null, 2));
  assert(results.every(r => r.report.issues.numErrors === 0 && r.report.issues.truncated !== true), 'official validator errors or truncated receipt');
})().catch(error => { console.error(error); process.exitCode = 1; });
