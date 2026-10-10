// Khronos core supplement; identity meaning is checked by f1c2_oracle.py.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const [modules, rootArgument] = process.argv.slice(2);
const root = fs.realpathSync(rootArgument);
const packageDirectory = path.resolve(modules, 'gltf-validator');
const validator = require(packageDirectory);
const hash = data => crypto.createHash('sha256').update(data).digest('hex');
(async () => {
  const results = [];
  for (const control of JSON.parse(fs.readFileSync(path.join(root, 'validation-plan.json')))) {
    const filename = fs.realpathSync(path.resolve(root, control.input));
    assert(filename.startsWith(root + path.sep));
    const bytes = fs.readFileSync(filename);
    const report = await validator.validateBytes(new Uint8Array(bytes), {
      uri: control.input, maxIssues: 0, writeTimestamp: false,
      externalResourceFunction: async uri => { throw new Error('Unexpected external URI ' + uri); },
    });
    assert(!report.issues.truncated);
    if (control.expectedCode) {
      assert(report.issues.messages.some(m => m.code === control.expectedCode), 'insensitive control ' + control.input);
    } else {
      assert.equal(report.issues.numErrors, 0, 'core errors ' + control.input);
      assert.equal(report.issues.numWarnings, 0, 'core warnings ' + control.input);
    }
    results.push({ ...control, sha256: hash(bytes), report });
  }
  const receipt = { validator_version: validator.version(), driver_sha256: hash(fs.readFileSync(__filename)),
    package_sha256: hash(fs.readFileSync(path.join(packageDirectory, 'package.json'))),
    entry_sha256: hash(fs.readFileSync(path.join(packageDirectory, 'index.js'))), results };
  console.log(JSON.stringify(receipt, null, 2));
})().catch(error => { console.error(error); process.exitCode = 1; });
